// interrupt.zig — 8259 PIC 中断控制器抽象
// 管理双片 8259 PIC (主片 + 从片) 的中断屏蔽、EOI 和路由
// IOAPIC 亲和性通过 IOAPIC 重定向表实现
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

// 主片 8259 I/O 端口
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;

// 从片 8259 I/O 端口
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

// PIC 命令字
const PIC_EOI: u8 = 0x20; // End-Of-Interrupt 命令

// IOAPIC 寄存器端口 (默认 MMIO 基地址由 ACPI/MADT 提供)
// 此处使用标准 I/O 端口间接访问方式作为后备
const IOAPIC_REGSEL: u16 = 0xFEC0; // IOAPIC 寄存器选择 (MMIO 偏移)
const IOAPIC_REGWIN: u16 = 0xFEC0 + 0x10; // IOAPIC 寄存器窗口

// IOAPIC 重定向表基地址寄存器
const IOAPIC_REDTBL_BASE: u8 = 0x10;

/// 向指定端口写入 8 位 I/O 数据
fn port_out8(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

/// 从指定端口读取 8 位 I/O 数据
fn port_in8(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}

/// 启用指定 IRQ 线 (清除屏蔽位)
/// IRQ 0-7 对应主片，IRQ 8-15 对应从片
export fn hal_irq_enable(irq: u8) void {
    if (irq < 8) {
        // 主片: 清除对应位以启用中断
        const mask = port_in8(PIC1_DATA);
        port_out8(PIC1_DATA, mask & ~(@as(u8, 1) << @intCast(irq)));
    } else {
        // 从片: IRQ 8-15 映射到从片位 0-7
        const mask = port_in8(PIC2_DATA);
        port_out8(PIC2_DATA, mask & ~(@as(u8, 1) << @intCast(irq - 8)));
    }
}

/// 禁用指定 IRQ 线 (设置屏蔽位)
/// IRQ 0-7 对应主片，IRQ 8-15 对应从片
export fn hal_irq_disable(irq: u8) void {
    if (irq < 8) {
        const mask = port_in8(PIC1_DATA);
        port_out8(PIC1_DATA, mask | (@as(u8, 1) << @intCast(irq)));
    } else {
        const mask = port_in8(PIC2_DATA);
        port_out8(PIC2_DATA, mask | (@as(u8, 1) << @intCast(irq - 8)));
    }
}

/// 发送 EOI (End Of Interrupt) 信号
/// IRQ 8-15 需同时向从片和主片发送 EOI (级联模式要求)
export fn hal_irq_eoi(irq: u8) void {
    if (irq >= 8) {
        // 从片 EOI
        port_out8(PIC2_COMMAND, PIC_EOI);
    }
    // 主片 EOI (所有 IRQ 都需要)
    port_out8(PIC1_COMMAND, PIC_EOI);
}

/// 设置双片 8259 PIC 的完整中断屏蔽掩码
/// 低 8 位对应主片 (IRQ 0-7)，高 8 位对应从片 (IRQ 8-15)
/// 位为 1 表示屏蔽 (禁用)，位为 0 表示启用
export fn hal_irq_set_mask(mask: u16) void {
    port_out8(PIC1_DATA, @intCast(mask & 0xFF));
    port_out8(PIC2_DATA, @intCast((mask >> 8) & 0xFF));
}

/// 获取双片 8259 PIC 的当前中断屏蔽掩码
/// 返回值的低 8 位为主片屏蔽字，高 8 位为从片屏蔽字
export fn hal_irq_get_mask() u16 {
    const master: u16 = port_in8(PIC1_DATA);
    const slave: u16 = port_in8(PIC2_DATA);
    return master | (slave << 8);
}

/// 设置指定 IRQ 的 CPU 亲和性 (将中断路由到特定 CPU)
/// 通过 IOAPIC 重定向表中的目标字段实现
/// irq 为 IRQ 编号，cpu 为目标 APIC ID
/// 注意: 需要 IOAPIC MMIO 已映射到虚拟地址空间
export fn hal_irq_set_affinity(irq: u8, cpu: u8) void {
    // 每个 IRQ 对应 IOAPIC 重定向表中的一个条目
    // 每个条目占两个 32 位寄存器 (低 32 位 + 高 32 位)
    const redir_reg: u8 = IOAPIC_REDTBL_BASE + irq * 2;

    // 读取当前重定向条目的低 32 位
    write_ioapic_reg(redir_reg, read_ioapic_reg(redir_reg));

    // 高 32 位包含目标 APIC ID (位 56-63，即寄存器位 24-31)
    const high_val: u32 = @as(u32, cpu) << 24;
    write_ioapic_reg(redir_reg + 1, high_val);
}

/// 向 MMIO 地址写入 32 位值（volatile）
fn mmio_write32(addr: u64, val: u32) void {
    const ptr: *volatile u32 = @ptrFromInt(addr);
    ptr.* = val;
}

/// 从 MMIO 地址读取 32 位值（volatile）
fn mmio_read32(addr: u64) u32 {
    const ptr: *volatile u32 = @ptrFromInt(addr);
    return ptr.*;
}

/// 写入 IOAPIC 间接寄存器
/// 先向 REGSEL 写入目标寄存器索引，再通过 REGWIN 写入数据
fn write_ioapic_reg(reg: u8, val: u32) void {
    // IOAPIC 基地址 0xFEC00000，REGSEL 偏移 0x00，REGWIN 偏移 0x10
    const regsel_addr: u64 = 0xFEC00000;
    const regwin_addr: u64 = 0xFEC00010;
    const reg32: u32 = @as(u32, @intCast(reg));

    mmio_write32(regsel_addr, reg32);
    mmio_write32(regwin_addr, val);
}

/// 读取 IOAPIC 间接寄存器
/// 先向 REGSEL 写入目标寄存器索引，再从 REGWIN 读取数据
fn read_ioapic_reg(reg: u8) u32 {
    const regsel_addr: u64 = 0xFEC00000;
    const regwin_addr: u64 = 0xFEC00010;
    const reg32: u32 = @as(u32, @intCast(reg));

    mmio_write32(regsel_addr, reg32);
    return mmio_read32(regwin_addr);
}
