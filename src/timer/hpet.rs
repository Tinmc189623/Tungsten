// timer/hpet.rs — HPET 高精度事件定时器探测
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

static mut HPET_OK: bool = false;

/// 探测 HPET 是否可用
pub fn probe() {
    unsafe {
        HPET_OK = false;
    }
    crate::serial::write_str(b"  timer: hpet probe\n");
}

/// HPET 是否可用
pub fn available() -> bool {
    unsafe { HPET_OK }
}

pub fn init() {}
