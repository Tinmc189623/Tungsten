// power/reboot.rs — 系统重启
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/// 通过键盘控制器执行冷重启
pub fn cold_reboot() -> ! {
    crate::serial::write_str(b"power: cold reboot\n");
    unsafe {
        loop {
            let mut status: u8;
            core::arch::asm!("in al, dx", out("al") status, in("dx") 0x64u16);
            if status & 0x02 == 0 {
                break;
            }
        }
        core::arch::asm!("out dx, al", in("dx") 0x64u16, in("al") 0xFEu8);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// 关机（ACPI S5，回退到 halt）
pub fn power_off() -> ! {
    crate::serial::write_str(b"power: halt (no ACPI S5)\n");
    unsafe {
        core::arch::asm!("cli");
    }
    loop {
        core::hint::spin_loop();
    }
}

pub fn init() {}
pub fn probe() {}
