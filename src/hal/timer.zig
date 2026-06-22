// timer.zig — 可编程间隔定时器 (i8254 PIT) 抽象
// 提供系统滴答计数和微秒/毫秒级延时，IRQ0 回调递增全局计数器
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

// PIT I/O 端口
const PIT_CHANNEL0: u16 = 0x40; // 通道 0 数据端口 (IRQ0)
const PIT_COMMAND: u16 = 0x43; // 命令寄存器

// PIT 基准振荡频率 (约 1.193182 MHz)
const PIT_FREQUENCY: u32 = 1193182;

// 默认目标频率 (1000 Hz = 1ms 滴答)
const TARGET_FREQUENCY: u32 = 1000;

// 命令字: 通道 0, 先低后高, 方波模式 (模式 3), 二进制计数
const PIT_CMD: u8 = 0x36;

// 全局滴答计数器，由 hal_timer_tick() 在 IRQ0 中断处理中递增
var tick_count: u64 = 0;

// 实际配置的分频值，用于精确延时计算
var configured_divisor: u16 = 0;

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

/// 初始化 PIT 通道 0 为方波模式，目标频率 1000 Hz
/// 每次定时器中断 (IRQ0) 触发时产生一个滴答
export fn hal_timer_init() void {
    const divisor: u16 = @intCast(PIT_FREQUENCY / TARGET_FREQUENCY);
    configured_divisor = divisor;

    // 发送命令字: 通道 0, 读写低高字节, 模式 3 (方波), 二进制
    port_out8(PIT_COMMAND, PIT_CMD);

    // 写入分频值低字节
    port_out8(PIT_CHANNEL0, @intCast(divisor & 0xFF));
    // 写入分频值高字节
    port_out8(PIT_CHANNEL0, @intCast((divisor >> 8) & 0xFF));

    // 重置滴答计数
    tick_count = 0;
}

/// 获取自 hal_timer_init() 以来的滴答计数
/// 该值由 IRQ0 中断处理程序调用 hal_timer_tick() 递增
export fn hal_timer_get_ticks() u64 {
    return tick_count;
}

/// 获取定时器实际频率 (Hz)
/// 返回值基于 PIT 基准频率和配置的分频器计算
export fn hal_timer_get_frequency() u32 {
    if (configured_divisor == 0) return TARGET_FREQUENCY;
    return PIT_FREQUENCY / @as(u32, configured_divisor);
}

/// 忙等待微秒级延时
/// 通过读取 PIT 通道 0 的当前计数值实现精确短时延时
/// 注意: 此函数会阻塞当前 CPU，仅用于硬件初始化等场景
export fn hal_timer_delay_us(us: u64) void {
    if (us == 0) return;

    // 将微秒转换为 PIT 计数值
    // PIT 频率约 1.193182 MHz，每计数约 0.838 微秒
    // counts = us * PIT_FREQUENCY / 1_000_000
    const counts = (us * PIT_FREQUENCY) / 1_000_000;
    if (counts == 0) return;

    // 使用 PIT 通道 0 锁存计数
    var remaining: u64 = counts;
    while (remaining > 0) {
        // 锁存通道 0 当前计数 (命令字 0x00)
        port_out8(PIT_COMMAND, 0x00);

        // 读取锁存值 (先低后高)
        const lo: u16 = port_in8(PIT_CHANNEL0);
        const hi: u16 = port_in8(PIT_CHANNEL0);
        const current: u16 = (hi << 8) | lo;

        if (current == 0) {
            // 计数器刚好归零，一个周期结束
            const period: u64 = if (configured_divisor > 0) configured_divisor else 65536;
            if (remaining <= period) break;
            remaining -= period;
        } else {
            // 等待计数值降低到接近 0
            // 忙等并重复锁存检查
            const elapsed: u64 = if (remaining > current) current else remaining;
            if (remaining <= elapsed) break;
            remaining -= elapsed;
        }

        asm volatile ("pause");
    }
}

/// 忙等待毫秒级延时
/// 基于滴答计数器实现，精度取决于定时器频率
export fn hal_timer_delay_ms(ms: u64) void {
    if (ms == 0) return;
    const freq = hal_timer_get_frequency();
    const target_ticks = tick_count + (ms * freq) / 1000;
    while (tick_count < target_ticks) {
        asm volatile ("hlt");
    }
}

/// 定时器中断处理入口 (由 Rust 中断分发器在 IRQ0 时调用)
/// 递增全局滴答计数器
export fn hal_timer_tick() void {
    tick_count += 1;
}
