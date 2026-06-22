// timer/tsc.rs — TSC 时间戳计数器校准与读取
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

const PIT_CH0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;

static TSC_KHZ: AtomicU64 = AtomicU64::new(0);
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

/// 读取 TSC
#[inline]
pub fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        asm!("rdtsc", out("eax") lo, out("edx") hi);
        ((hi as u64) << 32) | lo as u64
    }
}

/// 使用 PIT 校准 TSC 频率
pub fn calibrate() {
    unsafe {
        BOOT_TSC.store(rdtsc(), Ordering::Relaxed);

        asm!("out dx, al", in("al") 0x34u8, in("dx") PIT_CMD);
        asm!("out dx, al", in("al") 0x00u8, in("dx") PIT_CH0);
        asm!("out dx, al", in("al") 0x00u8, in("dx") PIT_CH0);

        let t0 = rdtsc();
        let mut status: u8 = 0;
        loop {
            asm!("in al, dx", out("al") status, in("dx") PIT_CH0);
            if status & 0x80 != 0 {
                break;
            }
        }
        let t1 = rdtsc();
        let delta = t1.wrapping_sub(t0);
        let khz = delta / 55;
        TSC_KHZ.store(khz.max(1), Ordering::Relaxed);

        crate::serial::write_str(b"tsc: ");
        crate::serial_put_u64(khz);
        crate::serial::write_str(b" kHz\n");
    }
}

/// TSC 频率 (kHz)
pub fn khz() -> u64 {
    TSC_KHZ.load(Ordering::Relaxed)
}

/// 自启动以来的毫秒数
pub fn now_ms() -> u64 {
    let khz = TSC_KHZ.load(Ordering::Relaxed);
    if khz == 0 {
        return 0;
    }
    let boot = BOOT_TSC.load(Ordering::Relaxed);
    rdtsc().saturating_sub(boot) / khz
}

pub fn init() {}
pub fn probe() {}
