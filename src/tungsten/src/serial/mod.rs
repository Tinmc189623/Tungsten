// serial/mod.rs — UART 16550 串口驱动 (COM1)
//
// 硬件 I/O 委托给 Zig HAL (hal::serial)，Rust 仅封装调用接口。
// COM1 基地址 0x3F8，波特率 115200。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/* ── Zig HAL FFI ── */

#[link(name = "hal_tungsten", kind = "static")]
unsafe extern "C" {
    /// 初始化串口硬件 (COM1, 115200 baud)
    #[link_name = "hal_serial_init_default"]
    fn hal_serial_init();
    /// 向串口输出字节缓冲区
    fn hal_serial_write_str(buf: *const u8, len: usize);
    /// 检测接收缓冲区是否有可读数据
    fn hal_serial_data_available() -> bool;
    /// 从串口读取一个字节 (忙等待)
    fn hal_serial_read_byte() -> u8;
}

/// 初始化 COM1 串口，调用 Zig HAL 完成硬件配置后输出就绪提示
pub fn init() {
    unsafe { hal_serial_init(); }
    write_str(b"serial: COM1 initialized (115200 baud)\n");
}

/// 发送字节切片到串口（\r\n 扩展由 Zig HAL 处理）
pub fn write_str(s: &[u8]) {
    unsafe { hal_serial_write_str(s.as_ptr(), s.len()); }
}

/// 检查串口接收缓冲区是否有数据
pub fn data_available() -> bool {
    unsafe { hal_serial_data_available() }
}

/// 从串口读取一个字节（忙等待直到数据到达）
pub fn read_byte() -> u8 {
    unsafe { hal_serial_read_byte() }
}

/// 同步读取一行；等待输入时主动让出 CPU（协作式多任务）
pub fn read_line(buf: &mut [u8]) -> usize {
    let mut i = 0;
    loop {
        if crate::sched::take_resched() {
            crate::sched::yield_now();
        }
        if !data_available() {
            crate::sched::yield_now();
            continue;
        }
        let c = read_byte();
        match c {
            b'\r' | b'\n' => {
                write_str(b"\r\n");
                break;
            }
            0x7F | 0x08 => {
                // 退格: 删除前一个字符并回显
                if i > 0 {
                    i -= 1;
                    write_str(b"\x08 \x08");
                }
            }
            c if c >= 0x20 && c < 0x7F => {
                // 可打印字符: 存入缓冲区并回显
                if i < buf.len() - 1 {
                    buf[i] = c;
                    i += 1;
                    write_str(&[c]);
                }
            }
            _ => {}
        }
    }
    buf[i] = 0; // null-terminate
    i
}

/// 发送格式化字符串到串口 (实现 core::fmt::Write)

pub fn write_fmt(args: core::fmt::Arguments) {
    use core::fmt::Write;
    struct SerialWriter;
    impl Write for SerialWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            crate::serial::write_str(s.as_bytes());
            Ok(())
        }
    }
    let _ = SerialWriter.write_fmt(args);
}

/// 输出 u64 值的十六进制表示到串口
pub fn write_hex(val: u64) {
    write_str(b"0x");
    let mut buf = [0u8; 16];
    let mut v = val;
    let mut i = 16usize;
    loop {
        i -= 1;
        let nibble = (v & 0xF) as u8;
        buf[i] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        v >>= 4;
        if v == 0 || i == 0 { break; }
    }
    write_str(&buf[i..]);
}

/// 输出 u64 值的十进制表示到串口
pub fn write_dec(val: u64) {
    if val == 0 {
        write_str(b"0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut v = val;
    let mut i = 20usize;
    while v > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    write_str(&buf[i..]);
}
