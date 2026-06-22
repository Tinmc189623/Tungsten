// devices/pci.rs — PCI 总线枚举与配置空间访问
//
// 使用 I/O 端口 0xCF8/0xCFC 访问 PCI 配置空间。
// 所有 PCI I/O 操作委托给 Zig HAL (hal_outd/hal_ind)。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



/* ── PCI I/O 端口 ── */

const PCI_CONFIG_ADDR: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

// Zig HAL I/O 端口接口
#[link(name = "hal_tungsten", kind = "static")]
unsafe extern "C" {
    /// 向 I/O 端口写 32 位
    fn hal_outd(port: u16, val: u32);
    /// 从 I/O 端口读 32 位
    fn hal_ind(port: u16) -> u32;
}

/* ── PCI 配置空间偏移 ── */

const PCI_VENDOR_ID: u8   = 0x00;
const PCI_DEVICE_ID: u8   = 0x02;
const PCI_COMMAND: u8     = 0x04;
const PCI_STATUS: u8      = 0x06;
const PCI_REVISION: u8    = 0x08;
const PCI_CLASS: u8       = 0x0A;
const PCI_HEADER_TYPE: u8 = 0x0E;
const PCI_BAR0: u8        = 0x10;

/* ── PCI 命令寄存器位 ── */

pub mod command {
    /// I/O 空间访问使能
    pub const IO_SPACE: u16     = 1 << 0;
    /// 内存空间访问使能
    pub const MEM_SPACE: u16    = 1 << 1;
    /// 总线主控使能 (DMA)
    pub const BUS_MASTER: u16   = 1 << 2;
}

/* ── PCI 配置空间访问 ── */

/// 构建 PCI 配置地址 (Type 1 配置事务)
fn pci_config_addr(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000u32
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// 读 PCI 配置空间 32 位
unsafe fn pci_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    hal_outd(PCI_CONFIG_ADDR, pci_config_addr(bus, dev, func, offset));
    hal_ind(PCI_CONFIG_DATA)
}

/// 写 PCI 配置空间 32 位

unsafe fn pci_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    hal_outd(PCI_CONFIG_ADDR, pci_config_addr(bus, dev, func, offset));
    hal_outd(PCI_CONFIG_DATA, val);
}

/// 读 PCI 配置空间 16 位
unsafe fn pci_read16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    (pci_read32(bus, dev, func, offset) >> ((offset as u32 & 2) * 8)) as u16
}

/// 读 PCI 配置空间 8 位
unsafe fn pci_read8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    (pci_read32(bus, dev, func, offset) >> ((offset as u32 & 3) * 8)) as u8
}

/* ── PCI 设备信息 ── */

/// PCI 设备描述
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub bars: [u32; 6],
    pub irq_pin: u8,
    pub irq_line: u8,
}

/// 已发现的 PCI 设备数量
static mut PCI_DEVICE_COUNT: usize = 0;
/// PCI 设备表 (最多 32 个)
static mut PCI_DEVICES: [PciDevice; 32] = [PciDevice {
    bus: 0, dev: 0, func: 0,
    vendor_id: 0, device_id: 0,
    class_code: 0, subclass: 0, prog_if: 0, revision: 0,
    bars: [0; 6], irq_pin: 0, irq_line: 0,
}; 32];

/// 返回发现的 PCI 设备数量
pub fn device_count() -> usize {
    unsafe { PCI_DEVICE_COUNT }
}

/// 返回 PCI 设备列表
pub fn devices() -> &'static [PciDevice] {
    unsafe { &PCI_DEVICES[..PCI_DEVICE_COUNT] }
}

/// 设置 PCI 命令寄存器 (IO/MEM/BUS_MASTER 位)
pub fn set_command(bus: u8, dev: u8, func: u8, cmd: u16) {
    unsafe {
        let addr = pci_config_addr(bus, dev, func, PCI_COMMAND);
        hal_outd(PCI_CONFIG_ADDR, addr);
        let mut val = hal_ind(PCI_CONFIG_DATA);
        val = (val & 0xFFFF_0000) | cmd as u32;
        hal_outd(PCI_CONFIG_ADDR, addr);
        hal_outd(PCI_CONFIG_DATA, val);
    }
}

/// 检查 PCI 设备是否为有效设备 (vendor != 0xFFFF 且 != 0x0000)
fn is_valid_device(vendor: u16, _device: u16) -> bool {
    vendor != 0xFFFF && vendor != 0x0000
}

/// 枚举指定总线上的所有设备 (含 PCI-PCI 桥递归)
fn check_bus(bus: u8) {
    for dev in 0..32 {
        let vendor = unsafe { pci_read16(bus, dev, 0, PCI_VENDOR_ID) };
        if !is_valid_device(vendor, 0) {
            continue;
        }

        // 遍历多功能设备的所有 function
        let max_funcs = if dev == 0 { 1 } else {
            let header = unsafe { pci_read8(bus, dev, 0, PCI_HEADER_TYPE) };
            if header & 0x80 != 0 { 8 } else { 1 }
        };

        for func in 0..max_funcs {
            let v = unsafe { pci_read16(bus, dev, func, PCI_VENDOR_ID) };
            if !is_valid_device(v, 0) {
                continue;
            }

            let device_id = unsafe { pci_read16(bus, dev, func, PCI_DEVICE_ID) };
            let class_raw = unsafe { pci_read16(bus, dev, func, PCI_CLASS) };
            let revision = unsafe { pci_read8(bus, dev, func, PCI_REVISION) };
            let prog_if = unsafe { pci_read8(bus, dev, func, 0x09) };
            let class_code = (class_raw >> 8) as u8;
            let subclass = (class_raw & 0xFF) as u8;

            // PCI-PCI 桥: 递归枚举次级总线
            if class_code == 0x06 && subclass == 0x04 {
                let secondary_bus = unsafe { pci_read8(bus, dev, func, 0x19) };
                check_bus(secondary_bus);
                continue;
            }

            // 读出所有 BAR
            let mut bars = [0u32; 6];
            for i in 0..6 {
                bars[i] = unsafe { pci_read32(bus, dev, func, PCI_BAR0 + (i as u8) * 4) };
            }

            let irq_pin = unsafe { pci_read8(bus, dev, func, 0x3D) };
            let irq_line = unsafe { pci_read8(bus, dev, func, 0x3C) };

            let count = unsafe { PCI_DEVICE_COUNT };
            if count >= 32 { continue; }

            unsafe {
                PCI_DEVICES[count] = PciDevice {
                    bus, dev, func,
                    vendor_id: v,
                    device_id,
                    class_code, subclass, prog_if, revision,
                    bars, irq_pin, irq_line,
                };
                PCI_DEVICE_COUNT += 1;
            }
        }
    }
}

/// 枚举所有 PCI 总线上的设备 (含多域检测)
pub fn enumerate_all() {
    // 检查 bus 0 存在性
    let vendor = unsafe { pci_read16(0, 0, 0, PCI_VENDOR_ID) };
    if !is_valid_device(vendor, 0) {
        return;
    }

    check_bus(0);

    // 检查是否有多个 PCI 域 (通过 bus 0 device 0 的 header type)
    let header = unsafe { pci_read8(0, 0, 0, PCI_HEADER_TYPE) };
    if header & 0x80 != 0 {
        // 多功能设备在 dev 0: 检查 function 1 是否指向另一条总线
        let vendor_1 = unsafe { pci_read16(0, 0, 1, PCI_VENDOR_ID) };
        if is_valid_device(vendor_1, 0) {
            let class_raw = unsafe { pci_read16(0, 0, 1, PCI_CLASS) };
            let class_code = (class_raw >> 8) as u8;
            let subclass = (class_raw & 0xFF) as u8;
            if class_code == 0x06 && subclass == 0x04 {
                let secondary_bus = unsafe { pci_read8(0, 0, 1, 0x19) };
                check_bus(secondary_bus);
            }
        }
    }

    // 启用所有设备的 bus master + IO + MMIO
    for i in 0..unsafe { PCI_DEVICE_COUNT } {
        let d = unsafe { &PCI_DEVICES[i] };
        set_command(d.bus, d.dev, d.func, command::IO_SPACE | command::MEM_SPACE | command::BUS_MASTER);
    }
}

/// 通过类别码查找第一个匹配的 PCI 设备
pub fn find_by_class(class_code: u8, subclass: u8) -> Option<&'static PciDevice> {
    for i in 0..device_count() {
        let d = unsafe { &PCI_DEVICES[i] };
        if d.class_code == class_code && d.subclass == subclass {
            return Some(d);
        }
    }
    None
}

/// 通过类别码与编程接口查找第一个匹配的 PCI 设备
pub fn find_by_class_prog(class_code: u8, subclass: u8, prog_if: u8) -> Option<&'static PciDevice> {
    for i in 0..device_count() {
        let d = unsafe { &PCI_DEVICES[i] };
        if d.class_code == class_code && d.subclass == subclass && d.prog_if == prog_if {
            return Some(d);
        }
    }
    None
}

/// 读 PCI 配置空间 32 位（按 BDF + 偏移）
pub fn config_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    unsafe { pci_read32(bus, dev, func, offset) }
}

/// 写 PCI 配置空间 32 位
pub fn config_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    unsafe { pci_write32(bus, dev, func, offset, val) }
}

/// 读 PCI 配置空间 16 位
pub fn config_read16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    unsafe { pci_read16(bus, dev, func, offset) }
}

/// 读 PCI 配置空间 8 位
pub fn config_read8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    unsafe { pci_read8(bus, dev, func, offset) }
}
