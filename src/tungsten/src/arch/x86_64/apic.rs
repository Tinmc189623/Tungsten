// arch/x86_64/apic.rs — x2APIC 本地中断控制器 + 定时器 + IOAPIC
// 管理 Local APIC、IOAPIC 路由、IRQ 屏蔽/解除、EOI
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::arch::x86_64::acpi;
use core::arch::asm;

/* ── MSR 地址（x2APIC 通过 MSR 访问） ── */

const IA32_APIC_BASE: u32         = 0x01B;
const IA32_X2APIC_APICID: u32     = 0x802;
const IA32_X2APIC_VERSION: u32    = 0x803;
const IA32_X2APIC_TPR: u32        = 0x808;
const IA32_X2APIC_EOI: u32        = 0x80B;
const IA32_X2APIC_SVR: u32        = 0x80F;
const IA32_X2APIC_TIMER: u32      = 0x832;
const IA32_X2APIC_LINT0: u32      = 0x835;
const IA32_X2APIC_LINT1: u32      = 0x836;
const IA32_X2APIC_ERROR: u32      = 0x837;
const IA32_X2APIC_INIT_COUNT: u32 = 0x838;
const IA32_X2APIC_CUR_COUNT: u32  = 0x839;
const IA32_X2APIC_DIV_CONFIG: u32 = 0x83E;
const IA32_X2APIC_ICR: u32        = 0x830;

/* ── IOAPIC MMIO 寄存器偏移 ── */

/// IOAPIC 寄存器选择端口偏移
const IOAPIC_IO_REG: u32 = 0x00;
/// IOAPIC 数据端口偏移
const IOAPIC_DATA_REG: u32 = 0x10;

/* ── 中断向量分配 ── */

/// IRQ 起始向量号（IRQ 0 = 向量 32）
pub const IRQ_BASE: u8 = 32;
/// 伪中断向量（Spurious Interrupt Vector）
pub const SPURIOUS_VECTOR: u8 = 0xFF;
/// APIC 定时器中断向量号
pub const TIMER_VECTOR: u8 = 32;

/* ── 缓存的 IOAPIC 信息 ── */

/// 缓存的 IOAPIC 内核虚拟地址（init 后有效）
static mut IOAPIC_VIRT: u64 = 0;
/// 缓存的 IOAPIC 最大 IRQ 引脚数
static mut IOAPIC_MAX_IRQ: u8 = 0;

/* ── MSR 读写 ── */

/// 读取 MSR 寄存器
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    asm!("rdmsr", out("eax") low, out("edx") high, in("ecx") msr);
    (low as u64) | ((high as u64) << 32)
}

/// 写入 MSR 寄存器
#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    asm!("wrmsr", in("eax") low, in("edx") high, in("ecx") msr);
}

/* ── IOAPIC MMIO 操作 ── */

/// IOAPIC MMIO 写操作（先选择寄存器，再写数据）
unsafe fn ioapic_write(addr: u64, reg: u8, val: u32) {
    let base = addr as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::write_volatile(base.add(1), val);
}

/// IOAPIC MMIO 读操作（先选择寄存器，再读数据）
unsafe fn ioapic_read(addr: u64, reg: u8) -> u32 {
    let base = addr as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::read_volatile(base.add(1))
}

/// 设置 IOAPIC 重定向表项（RTE），将硬件 IRQ 映射到指定向量号和 CPU
///
/// RTE 由两个 32 位寄存器组成（低 32 位和高 32 位）。
/// 低 32 位包含向量号、投递模式、触发模式、屏蔽位等。
/// 高 32 位包含目标 CPU 的 LAPIC ID。
unsafe fn ioapic_set_irq(ioapic_addr: u64, irq: u8, vector: u8, cpu: u8) {
    let reg_index = 0x10 + (irq as u8) * 2;
    let low: u32 = vector as u32    // Vector [7:0]
        | (0 << 8)                  // Delivery Mode: Fixed
        | (0 << 11)                 // Destination Mode: Physical
        | (0 << 15)                 // Remote IRR: 0
        | (0 << 16)                 // Interrupt Mask: unmasked (0=unmasked)
        | (0 << 17);                // Trigger Mode: edge (0=edge)
    let high: u32 = (cpu as u32) << 24; // Destination: CPU LAPIC ID
    ioapic_write(ioapic_addr, reg_index, low);
    ioapic_write(ioapic_addr, reg_index + 1, high);
}

/* ── 定时器校准 ── */

/// PIT I/O 端口
const PIT_CH0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;

/// 使用 PIT 校准 APIC 定时器频率
///
/// PIT 通道 0 产生已知时间间隔，通过比较 APIC 定时器计数值
/// 计算每毫秒的 tick 数，然后配置定时器为周期性模式（每 1ms 触发一次）。
fn calibrate_apic_timer() -> u32 {
    unsafe {
        // PIT 通道 0，模式 2（速率发生器），计数值 0 = 65536
        asm!("out dx, al", in("al") 0x34u8, in("dx") PIT_CMD);
        asm!("out dx, al", in("al") 0x00u8, in("dx") PIT_CH0);
        asm!("out dx, al", in("al") 0x00u8, in("dx") PIT_CH0);

        // APIC 定时器：分频 16, 最大初始计数值
        wrmsr(IA32_X2APIC_DIV_CONFIG, 3);
        wrmsr(IA32_X2APIC_INIT_COUNT, 0xFFFFFFFF);

        // 发送 PIT 读回命令（计数器 0 状态锁存）
        asm!("out dx, al", in("al") 0xE4u8, in("dx") PIT_CMD);
        // 等待一个 PIT 周期完成
        let mut status: u8;
        loop {
            asm!("in al, dx", out("al") status, in("dx") PIT_CH0);
            if status & 0x80 != 0 {
                break;
            }
        }

        // 计算每毫秒 APIC 计数值
        let current = rdmsr(IA32_X2APIC_CUR_COUNT) as u32;
        let elapsed = 0xFFFFFFFFu32.wrapping_sub(current);
        let per_ms = elapsed / 55; // PIT 一个周期约 55ms

        // 设置定时器为周期性模式（bit 17 = Periodic），每 1ms 触发
        wrmsr(IA32_X2APIC_TIMER, TIMER_VECTOR as u64 | (1 << 17));
        wrmsr(IA32_X2APIC_DIV_CONFIG, 3);
        wrmsr(IA32_X2APIC_INIT_COUNT, per_ms as u64);

        per_ms
    }
}

/* ── 初始化 ── */

/// 初始化 APIC 子系统
///
/// 1. 解析 ACPI 获取 LAPIC/IOAPIC 信息
/// 2. 启用 x2APIC 模式
/// 3. 设置伪中断向量 (SVR)
/// 4. 屏蔽 LINT0/LINT1/Error 中断
/// 5. 初始化 IOAPIC 路由表
/// 6. 校准并启动 APIC 定时器
///
/// # Safety
/// 必须在 IDT 初始化之后调用，且只调用一次。
pub unsafe fn init(rsdp_addr: u64) {
    // 解析 ACPI 获取 MADT/IOAPIC 信息
    let acpi_info = acpi::parse(rsdp_addr);

    // ── 启用 x2APIC ──
    let apic_base = rdmsr(IA32_APIC_BASE);
    if apic_base & (1 << 10) == 0 {
        // 设置 bit 10 (x2APIC Enable) 和 bit 11 (APIC Global Enable)
        wrmsr(IA32_APIC_BASE, apic_base | (1 << 10) | (1 << 11));
    }

    // ── 设置伪中断向量 ──
    // SVR bit 8 = APIC Software Enable
    wrmsr(IA32_X2APIC_SVR, SPURIOUS_VECTOR as u64 | (1 << 8));

    // ── 屏蔽 LINT0/LINT1 ──
    wrmsr(IA32_X2APIC_LINT0, 1 << 16); // mask
    wrmsr(IA32_X2APIC_LINT1, 1 << 16); // mask

    // ── 屏蔽错误中断 ──
    wrmsr(IA32_X2APIC_ERROR, 1 << 16);

    // ── 初始化 IOAPIC ──
    if let Some(ref info) = acpi_info {
        for ioapic in info.ioapics.iter().take(info.ioapic_count) {
            if IOAPIC_VIRT == 0 {
                let virt = crate::mm::vmm::map_mmio(ioapic.addr as u64, 0x1000);
                IOAPIC_VIRT = virt;
                let ver = ioapic_read(virt, 0x01);
                IOAPIC_MAX_IRQ = ((ver >> 16) & 0xFF) as u8;
                crate::serial::write_str(b"  apic: IOAPIC @ 0x");
                crate::serial_put_u64_hex(virt);
                crate::serial::write_str(b" max_irq=");
                crate::serial_put_u64(IOAPIC_MAX_IRQ as u64);
                crate::serial::write_str(b"\n");
                for irq in 0..=IOAPIC_MAX_IRQ {
                    let vector = IRQ_BASE.saturating_add(irq);
                    ioapic_set_irq(virt, irq, vector, 0);
                }
                // 解除键盘中断 (IRQ1)
                unmask_irq(1);
            }
        }
    }

    // ── 校准并启动 APIC 定时器 ──
    calibrate_apic_timer();
}

/// 启用 APIC（确保 APIC 全局使能位已设置）
///
/// 重新写入 SVR 的使能位，确保 APIC 处于活跃状态。
/// 通常在 `init()` 之后或从节能状态恢复后调用。
pub fn enable() {
    unsafe {
        let apic_base = rdmsr(IA32_APIC_BASE);
        if apic_base & (1 << 11) == 0 {
            wrmsr(IA32_APIC_BASE, apic_base | (1 << 11));
        }
        // 确保 SVR 使能
        wrmsr(IA32_X2APIC_SVR, SPURIOUS_VECTOR as u64 | (1 << 8));
    }
}

/// 发送 EOI（End of Interrupt）
///
/// 通知 Local APIC 当前中断已处理完毕，允许下一个中断投递。
/// 必须在每个 IRQ 处理程序末尾调用。
#[inline]
pub fn eoi() {
    unsafe {
        wrmsr(IA32_X2APIC_EOI, 0);
    }
}

/// 读取当前 LAPIC ID
#[inline]
pub fn lapic_id() -> u32 {
    unsafe { rdmsr(IA32_X2APIC_APICID) as u32 }
}

/// 向目标 APIC ID 发送核间中断 (IPI)
///
/// `dest_apic_id` 为目标 Local APIC ID，`vector` 为投递的中断向量。
pub fn send_ipi(dest_apic_id: u32, vector: u8) {
    unsafe {
        let high = (dest_apic_id as u64) << 32;
        let low = vector as u64;
        wrmsr(IA32_X2APIC_ICR, high | low);
    }
}

/// 读取 APIC 定时器当前计数值
#[inline]
pub fn timer_current_count() -> u32 {
    unsafe { rdmsr(IA32_X2APIC_CUR_COUNT) as u32 }
}

/// 屏蔽指定 IRQ（设置 IOAPIC RTE 的 Mask 位）
///
/// `irq` 为硬件 IRQ 编号（0-23），对应 IOAPIC 引脚。
/// 屏蔽后该 IRQ 不再投递到 CPU。
pub fn mask_irq(irq: u8) {
    unsafe {
        if IOAPIC_VIRT == 0 {
            return;
        }
        let reg_index = 0x10 + (irq as u32) * 2;
        let low = ioapic_read(IOAPIC_VIRT, reg_index as u8);
        ioapic_write(IOAPIC_VIRT, reg_index as u8, low | (1 << 16));
    }
}

/// 解除屏蔽指定 IRQ（清除 IOAPIC RTE 的 Mask 位）
pub fn unmask_irq(irq: u8) {
    unsafe {
        if IOAPIC_VIRT == 0 {
            return;
        }
        let reg_index = 0x10 + (irq as u32) * 2;
        let low = ioapic_read(IOAPIC_VIRT, reg_index as u8);
        ioapic_write(IOAPIC_VIRT, reg_index as u8, low & !(1 << 16));
    }
}
