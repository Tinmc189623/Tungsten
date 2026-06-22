// serial.zig — UART 16550 串口驱动
// 支持 COM1~COM4，提供可配置波特率的字节/缓冲/行级读写
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

// UART 16550 寄存器偏移
const RBR: u16 = 0; // 接收缓冲寄存器 (读)
const THR: u16 = 0; // 发送保持寄存器 (写)
const IER: u16 = 1; // 中断使能寄存器
const FCR: u16 = 2; // FIFO 控制寄存器
const LCR: u16 = 3; // 线路控制寄存器
const MCR: u16 = 4; // 调制解调器控制寄存器
const LSR: u16 = 5; // 线路状态寄存器
const DLL: u16 = 0; // 分频器低字节 (DLAB=1)
const DLH: u16 = 1; // 分频器高字节 (DLAB=1)

// 线路状态位
const LSR_DR: u8 = 0x01; // 数据就绪
const LSR_THRE: u8 = 0x20; // 发送保持寄存器空

// UART 基准时钟频率 (1.8432 MHz)
const UART_CLOCK: u32 = 115200;

/// 从指定端口读取 8 位 I/O 数据
fn port_in(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}

/// 向指定端口写入 8 位 I/O 数据
fn port_out(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

/// 根据目标波特率计算分频器值
/// UART 时钟为 1.8432MHz，分频器 = 115200 / baud
fn calc_divisor(baud: u32) u16 {
    if (baud == 0) return 1;
    const d = UART_CLOCK / baud;
    return if (d == 0) 1 else @intCast(d);
}

/// 初始化指定 COM 端口，设置波特率、8N1 数据格式、启用 FIFO
/// 支持 COM1(0x3F8), COM2(0x2F8), COM3(0x3E8), COM4(0x2E8)
export fn hal_serial_init(port: u16, baud: u32) void {
    const divisor = calc_divisor(baud);

    // 禁用所有中断
    port_out(port + IER, 0x00);

    // 启用 DLAB 以设置波特率分频器
    port_out(port + LCR, 0x80);

    // 写入分频器低字节和高字节
    port_out(port + DLL, @intCast(divisor & 0xFF));
    port_out(port + DLH, @intCast((divisor >> 8) & 0xFF));

    // 8 位数据位，无奇偶校验，1 位停止位 (8N1)，关闭 DLAB
    port_out(port + LCR, 0x03);

    // 启用 FIFO，清空收发缓冲区，14 字节阈值
    port_out(port + FCR, 0xC7);

    // 设置 DTR、RTS 和 OUT2 (中断输出使能)
    port_out(port + MCR, 0x0B);
}

/// 向指定 COM 端口写入单字节，忙等待直到发送保持寄存器就绪
export fn hal_serial_write(port: u16, byte: u8) void {
    // 等待发送保持寄存器空
    while ((port_in(port + LSR) & LSR_THRE) == 0) {
        asm volatile ("pause");
    }
    port_out(port + THR, byte);
}

/// 从指定 COM 端口读取单字节，忙等待直到数据就绪
export fn hal_serial_read(port: u16) u8 {
    // 等待数据就绪
    while ((port_in(port + LSR) & LSR_DR) == 0) {
        asm volatile ("pause");
    }
    return port_in(port + RBR);
}

/// 将缓冲区中的 len 字节写入指定 COM 端口
/// 自动将 \n 扩展为 \r\n 以兼容终端
export fn hal_serial_write_buf(port: u16, buf: [*]const u8, len: usize) void {
    var i: usize = 0;
    while (i < len) : (i += 1) {
        const byte = buf[i];
        if (byte == '\n') {
            hal_serial_write(port, '\r');
        }
        hal_serial_write(port, byte);
    }
}

/// 从指定 COM 端口读取一行到缓冲区
/// 以 \n 或缓冲区满为终止条件，返回实际读取的字节数 (不含终止符)
export fn hal_serial_read_line(port: u16, buf: [*]u8, max: usize) usize {
    var count: usize = 0;
    while (count < max) {
        const byte = hal_serial_read(port);
        // 遇到换行符终止
        if (byte == '\n') break;
        // 忽略回车符
        if (byte == '\r') continue;
        buf[count] = byte;
        count += 1;
    }
    return count;
}

/// 检查指定 COM 端口是否有数据可读
/// 返回 true 表示接收缓冲区中有待读取的数据
export fn hal_serial_available(port: u16) bool {
    return (port_in(port + LSR) & LSR_DR) != 0;
}

/// 运行时重新设置指定 COM 端口的波特率
/// 保持当前的 8N1 数据格式不变
export fn hal_serial_set_baud(port: u16, baud: u32) void {
    const divisor = calc_divisor(baud);

    // 启用 DLAB
    port_out(port + LCR, 0x80);

    // 写入新分频器
    port_out(port + DLL, @intCast(divisor & 0xFF));
    port_out(port + DLH, @intCast((divisor >> 8) & 0xFF));

    // 恢复 8N1 格式，关闭 DLAB
    port_out(port + LCR, 0x03);
}

// ── Rust 兼容层 (COM1 0x3F8 默认端口) ──

const COM1: u16 = 0x3F8;

/// Rust 侧无参数初始化 (COM1, 115200 baud)
export fn hal_serial_init_default() void {
    hal_serial_init(COM1, 115200);
}

/// Rust 侧 hal_serial_write_str(buf, len) — 写入字节缓冲到 COM1
export fn hal_serial_write_str(buf: [*]const u8, len: usize) void {
    hal_serial_write_buf(COM1, buf, len);
}

/// Rust 侧 hal_serial_data_available() — 检查 COM1 是否有数据
export fn hal_serial_data_available() bool {
    return hal_serial_available(COM1);
}

/// Rust 侧 hal_serial_read_byte() — 从 COM1 读取单字节
export fn hal_serial_read_byte() u8 {
    return hal_serial_read(COM1);
}
