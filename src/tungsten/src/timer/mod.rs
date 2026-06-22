// timer/mod.rs — 高精度定时器子系统 (HPET/PIT/APIC Timer/TSC)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apic_timer;
pub mod hpet;
pub mod pit;
pub mod tsc;

use crate::sync::SpinLock;

/// 内核定时器对象
pub struct Timer {
    pub id: u64,
    pub expires: u64,
    pub interval: u64,
    pub callback: *const (),
    pub arg: *mut (),
    pub flags: u32,
    pub next: *mut Timer,
}

/// 定时器轮
pub struct TimerWheel {
    pub buckets: [*mut Timer; 8],
    pub count: usize,
    pub now: u64,
}

/// 定时器管理器
pub struct TimerManager {
    pub wheel: TimerWheel,
    pub hpet_available: bool,
    pub tsc_khz: u64,
}

unsafe impl Send for TimerManager {}

static TIMER_MGR: SpinLock<TimerManager> = SpinLock::new(TimerManager {
    wheel: TimerWheel {
        buckets: [core::ptr::null_mut(); 8],
        count: 0,
        now: 0,
    },
    hpet_available: false,
    tsc_khz: 0,
});

/// 初始化定时器子系统
pub fn init() {
    hpet::probe();
    pit::init();
    tsc::calibrate();
    let khz = tsc::khz();
    let mut mgr = TIMER_MGR.lock();
    mgr.tsc_khz = khz;
    mgr.hpet_available = hpet::available();
    mgr.wheel.now = tsc::now_ms();
    crate::serial::write_str(b"timer: subsystem ready\n");
}

/// POSIX nanosleep
pub fn sys_nanosleep(_req: u64, _rem: u64) -> i32 {
    0
}

/// 系统运行毫秒数
pub fn uptime_ms() -> u64 {
    let ms = tsc::now_ms();
    TIMER_MGR.lock().wheel.now = ms;
    ms
}

/// 刷新定时器轮当前时间
pub fn tick_update() {
    TIMER_MGR.lock().wheel.now = tsc::now_ms();
}
