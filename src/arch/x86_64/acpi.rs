// arch/x86_64/acpi.rs — ACPI 表解析（RSDP → XSDT/RSDT → MADT 及其他）
// 支持 ACPI v1.0 (RSDT/32-bit) 和 v2.0+ (XSDT/64-bit)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



/* ── ACPI 结构体 ── */

/// ACPI RSDP（Root System Description Pointer）
///
/// BIOS 在 E820 区域或 UEFI Configuration Table 中提供此结构的物理地址。
/// ACPI v2.0+ 扩展了 XSDT 64 位地址。
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    /* ACPI v2.0+ 扩展字段 */
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    _reserved: [u8; 3],
}

/// ACPI SDT 通用头（System Description Table Header）
///
/// 所有 ACPI 表（MADT、HPET、MCFG 等）共享此 36 字节头部。
#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// MADT 中的 IOAPIC 条目（Type = 1）
#[repr(C, packed)]
struct MadtIoApic {
    _type: u8,
    _length: u8,
    ioapic_id: u8,
    _reserved: u8,
    ioapic_addr: u32,
    gsi_base: u32,
}

/// MADT 中的 Local APIC 条目（Type = 0）
#[repr(C, packed)]
struct MadtLocalApic {
    _type: u8,
    _length: u8,
    _acpi_processor_id: u8,
    apic_id: u8,
    flags: u32,
}

/// MADT 中的 Local x2APIC 条目（Type = 9）
#[repr(C, packed)]
struct MadtX2Apic {
    _type: u8,
    _length: u8,
    _reserved: [u8; 2],
    apic_id: u32,
    flags: u32,
    _acpi_processor_uid: u32,
}

/// MADT 中的 Interrupt Source Override 条目（Type = 2）
#[repr(C, packed)]
struct MadtIso {
    _type: u8,
    _length: u8,
    bus: u8,
    source: u8,
    gsi: u32,
    flags: u16,
}

/// MADT 表头（紧跟 SdtHeader 之后）
#[repr(C, packed)]
struct Madt {
    header: SdtHeader,
    lapic_addr: u32,
    flags: u32,
}

/* ── 解析结果 ── */

/// IOAPIC 信息（从 MADT Type 1 条目提取）
#[derive(Clone, Copy)]
pub struct IoApicInfo {
    /// IOAPIC ID
    pub id: u8,
    /// MMIO 基地址
    pub addr: u32,
    /// 全局系统中断基号
    pub gsi_base: u32,
}

/// ACPI 解析结果汇总
pub struct AcpiInfo {
    /// Local APIC MMIO 基地址
    pub lapic_addr: u32,
    /// 检测到的 IOAPIC 列表（最多 8 个）
    pub ioapics: [IoApicInfo; 8],
    /// IOAPIC 数量
    pub ioapic_count: usize,
}

/* ── 全局缓存 ── */

/// 缓存的 XSDT/RSDT 物理地址（init 后有效）
static mut ACPI_XSDT_ADDR: u64 = 0;
/// 缓存的 XSDT 条目数量
static mut ACPI_XSDT_COUNT: usize = 0;
/// 是否为 64 位 XSDT（false 则为 32 位 RSDT）
static mut ACPI_IS_XSDT: bool = false;

/* ── 校验 ── */

/// ACPI 表校验和检查：所有字节之和应为 0（mod 256）
fn checksum(table: *const u8, len: usize) -> bool {
    let mut sum: u8 = 0;
    for i in 0..len {
        unsafe {
            sum = sum.wrapping_add(*table.add(i));
        }
    }
    sum == 0
}

/* ── MADT 解析 ── */

/// 解析 MADT 表，提取 LAPIC 地址和 IOAPIC 信息
unsafe fn parse_madt(madt: *const u8) -> Option<AcpiInfo> {
    let madt_len = core::ptr::read_unaligned(madt.add(4) as *const u32) as usize;
    // Madt.lapic_addr 在 SdtHeader(36 bytes) 之后
    let lapic_addr = core::ptr::read_unaligned(madt.add(36) as *const u32);

    let mut info = AcpiInfo {
        lapic_addr,
        ioapics: [IoApicInfo {
            id: 0,
            addr: 0,
            gsi_base: 0,
        }; 8],
        ioapic_count: 0,
    };

    // 44 = sizeof(SdtHeader) + sizeof(lapic_addr) + sizeof(flags)
    let mut offset: usize = 44;
    while offset + 2 <= madt_len {
        let entry = madt.add(offset);
        let entry_type = *entry;
        let entry_len = *entry.add(1) as usize;
        if entry_len < 2 {
            break;
        }

        if entry_type == 1 && entry_len >= 12 && info.ioapic_count < 8 {
            // IOAPIC 条目
            let id = *entry.add(2);
            let addr = core::ptr::read_unaligned(entry.add(4) as *const u32);
            let gsi_base = core::ptr::read_unaligned(entry.add(8) as *const u32);
            info.ioapics[info.ioapic_count] = IoApicInfo {
                id,
                addr,
                gsi_base,
            };
            info.ioapic_count += 1;
        }

        offset += entry_len;
    }

    Some(info)
}

/* ── 公开 API ── */

/// 初始化 ACPI 子系统，验证 RSDP 并缓存 XSDT/RSDT 信息
///
/// `rsdp_addr` 为 Limine 提供的 RSDP 物理地址。
/// 验证 RSDP 签名 "RSD PTR " 后，缓存 XSDT/RSDT 地址供后续 `find_table` 使用。
///
/// # Safety
/// `rsdp_addr` 必须指向有效的 RSDP 结构。
pub unsafe fn init(rsdp_addr: u64) {
    let rsdp = rsdp_addr as *const u8;

    // 验证 RSDP 签名
    let sig = core::ptr::read_unaligned(rsdp as *const [u8; 8]);
    if &sig != b"RSD PTR " {
        crate::serial::write_str(b"  acpi: RSDP signature mismatch\n");
        return;
    }

    let revision = *rsdp.add(15);

    if revision >= 2 {
        // ACPI v2.0+: 使用 XSDT (64-bit 地址)
        let xsdt_addr = core::ptr::read_unaligned(rsdp.add(24) as *const u64);
        if xsdt_addr != 0 {
            let sdt = xsdt_addr as *const u8;
            let sdt_len = core::ptr::read_unaligned(sdt.add(4) as *const u32) as usize;
            if checksum(sdt, sdt_len) {
                let header_size = core::mem::size_of::<SdtHeader>();
                ACPI_XSDT_ADDR = xsdt_addr;
                ACPI_XSDT_COUNT = (sdt_len - header_size) / 8;
                ACPI_IS_XSDT = true;
                crate::serial::write_str(b"  acpi: XSDT found, entries=");
                crate::serial_put_u64(ACPI_XSDT_COUNT as u64);
                crate::serial::write_str(b"\n");
                return;
            }
        }
    }

    // 回退到 RSDT (32-bit 地址)
    let rsdt_addr = core::ptr::read_unaligned(rsdp.add(16) as *const u32) as u64;
    if rsdt_addr != 0 {
        let sdt = rsdt_addr as *const u8;
        let sdt_len = core::ptr::read_unaligned(sdt.add(4) as *const u32) as usize;
        if checksum(sdt, sdt_len) {
            let header_size = core::mem::size_of::<SdtHeader>();
            ACPI_XSDT_ADDR = rsdt_addr;
            ACPI_XSDT_COUNT = (sdt_len - header_size) / 4;
            ACPI_IS_XSDT = false;
            crate::serial::write_str(b"  acpi: RSDT found, entries=");
            crate::serial_put_u64(ACPI_XSDT_COUNT as u64);
            crate::serial::write_str(b"\n");
            return;
        }
    }

    crate::serial::write_str(b"  acpi: no valid XSDT/RSDT found\n");
}

/// 在 XSDT/RSDT 中查找指定签名的 ACPI 表
///
/// `sig` 为 4 字节签名，如 b"APIC" (MADT)、b"HPET"、b"MCFG" 等。
/// 返回表的物理地址，未找到返回 None。
///
/// # Safety
/// 必须在 `init()` 之后调用。返回的地址指向 ACPI 固件内存。
pub unsafe fn find_table(sig: &[u8; 4]) -> Option<u64> {
    if ACPI_XSDT_ADDR == 0 || ACPI_XSDT_COUNT == 0 {
        return None;
    }

    let sdt = ACPI_XSDT_ADDR as *const u8;
    let header_size = core::mem::size_of::<SdtHeader>();

    for i in 0..ACPI_XSDT_COUNT {
        let entry_addr = if ACPI_IS_XSDT {
            // XSDT: 每项 8 字节 (u64)
            core::ptr::read_unaligned(sdt.add(header_size).cast::<u64>().add(i))
        } else {
            // RSDT: 每项 4 字节 (u32)
            core::ptr::read_unaligned(sdt.add(header_size).cast::<u32>().add(i)) as u64
        };

        if entry_addr == 0 {
            continue;
        }

        let tbl = entry_addr as *const u8;
        let tbl_sig = core::ptr::read_unaligned(tbl as *const [u8; 4]);
        if &tbl_sig == sig {
            return Some(entry_addr);
        }
    }

    None
}

/// 解析 ACPI 表，返回 MADT 信息（IOAPIC 等）
///
/// 这是 `init()` + `find_table(b"APIC")` + `parse_madt()` 的便捷组合。
///
/// # Safety
/// `rsdp_addr` 必须指向有效的 RSDP 描述符。
pub unsafe fn parse(rsdp_addr: u64) -> Option<AcpiInfo> {
    let rsdp = rsdp_addr as *const u8;

    // 验证 RSDP 签名
    let sig = core::ptr::read_unaligned(rsdp as *const [u8; 8]);
    if &sig != b"RSD PTR " {
        return None;
    }

    // 获取 SDT 地址
    let revision = *rsdp.add(15);
    let sdt_addr: u64;
    if revision >= 2 {
        sdt_addr = core::ptr::read_unaligned(rsdp.add(24) as *const u64);
    } else {
        sdt_addr = core::ptr::read_unaligned(rsdp.add(16) as *const u32) as u64;
    }
    if sdt_addr == 0 {
        return None;
    }

    let sdt = sdt_addr as *const u8;
    let sdt_len = core::ptr::read_unaligned(sdt.add(4) as *const u32) as usize;
    if !checksum(sdt, sdt_len) {
        return None;
    }

    // 遍历 SDT 查找 MADT ("APIC")
    let header_size = core::mem::size_of::<SdtHeader>();
    let is_xsdt = revision >= 2;
    let entry_size = if is_xsdt { 8 } else { 4 };
    let entry_count = (sdt_len - header_size) / entry_size;

    for i in 0..entry_count {
        let entry_addr = if is_xsdt {
            core::ptr::read_unaligned(sdt.add(header_size).cast::<u64>().add(i))
        } else {
            core::ptr::read_unaligned(sdt.add(header_size).cast::<u32>().add(i)) as u64
        };
        if entry_addr == 0 {
            continue;
        }

        let tbl = entry_addr as *const u8;
        let tbl_sig = core::ptr::read_unaligned(tbl as *const [u8; 4]);
        if &tbl_sig == b"APIC" {
            return parse_madt(tbl);
        }
    }
    None
}

/// 从 MADT 枚举已启用的 Local APIC ID
///
/// 返回写入 `out` 的 CPU 数量；`out[i]` 为 APIC ID。
///
/// # Safety
/// 必须在 `init()` 之后调用。
pub unsafe fn enumerate_cpus(out: &mut [u32]) -> usize {
    let Some(madt_addr) = find_table(b"APIC") else {
        if out.is_empty() {
            return 0;
        }
        out[0] = crate::arch::x86_64::apic::lapic_id();
        return 1;
    };
    let madt = madt_addr as *const u8;
    let madt_len = core::ptr::read_unaligned(madt.add(4) as *const u32) as usize;
    let mut count = 0usize;
    let mut offset: usize = 44;
    while offset + 2 <= madt_len && count < out.len() {
        let entry = madt.add(offset);
        let entry_type = *entry;
        let entry_len = *entry.add(1) as usize;
        if entry_len < 2 {
            break;
        }
        if entry_type == 0 && entry_len >= 8 {
            let lapic = core::ptr::read_unaligned(entry as *const MadtLocalApic);
            if lapic.flags & 1 != 0 {
                out[count] = lapic.apic_id as u32;
                count += 1;
            }
        } else if entry_type == 9 && entry_len >= 16 {
            let x2 = core::ptr::read_unaligned(entry as *const MadtX2Apic);
            if x2.flags & 1 != 0 {
                out[count] = x2.apic_id;
                count += 1;
            }
        }
        offset += entry_len;
    }
    if count == 0 && !out.is_empty() {
        out[0] = crate::arch::x86_64::apic::lapic_id();
        count = 1;
    }
    count
}
