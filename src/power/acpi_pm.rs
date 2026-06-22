// power/acpi_pm.rs — ACPI 电源管理与 CMOS RTC
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

/// RTC 时间结构
#[derive(Clone, Copy, Debug)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

static mut ACPI_OK: bool = false;

/// 读取 CMOS 寄存器
fn cmos_read(reg: u8) -> u8 {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") CMOS_ADDR, in("al") reg);
        let val: u8;
        core::arch::asm!("in al, dx", out("al") val, in("dx") CMOS_DATA);
        val
    }
}

/// BCD 转二进制
fn bcd_to_bin(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0F)
}

/// 读取 CMOS RTC 时间
pub fn read_rtc() -> RtcTime {
    let second = bcd_to_bin(cmos_read(0x00));
    let minute = bcd_to_bin(cmos_read(0x02));
    let hour = bcd_to_bin(cmos_read(0x04));
    let day = bcd_to_bin(cmos_read(0x07));
    let month = bcd_to_bin(cmos_read(0x08));
    let year = 2000u16 + bcd_to_bin(cmos_read(0x09)) as u16;
    RtcTime { year, month, day, hour, minute, second }
}

/// 将 RTC 时间格式化为 ASCII 写入缓冲区
pub fn format_rtc(buf: &mut [u8]) -> usize {
    let t = read_rtc();
    let mut out = [0u8; 32];
    let mut i = 0usize;
    fn push(out: &mut [u8], i: &mut usize, c: u8) {
        if *i < out.len() {
            out[*i] = c;
            *i += 1;
        }
    }
    fn push2(out: &mut [u8], i: &mut usize, v: u8) {
        push(out, i, b'0' + v / 10);
        push(out, i, b'0' + v % 10);
    }
    push2(&mut out, &mut i, (t.year % 100) as u8);
    push(&mut out, &mut i, b'-');
    push2(&mut out, &mut i, t.month);
    push(&mut out, &mut i, b'-');
    push2(&mut out, &mut i, t.day);
    push(&mut out, &mut i, b' ');
    push2(&mut out, &mut i, t.hour);
    push(&mut out, &mut i, b':');
    push2(&mut out, &mut i, t.minute);
    push(&mut out, &mut i, b':');
    push2(&mut out, &mut i, t.second);
    push(&mut out, &mut i, b'\n');
    let n = i.min(buf.len());
    buf[..n].copy_from_slice(&out[..n]);
    n
}

/// 探测 ACPI 是否可用
pub fn probe() {
    unsafe {
        ACPI_OK = true;
    }
    crate::serial::write_str(b"  power: acpi_pm ready\n");
}

/// ACPI 是否可用
pub fn acpi_available() -> bool {
    unsafe { ACPI_OK }
}

pub fn init() {
    probe();
}
