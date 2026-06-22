// watchdog/mod.rs — 硬件看门狗定时器
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub struct WdogDevice { pub timeout_sec: u16, pub remaining_sec: u16, pub enabled: bool }
pub struct WdogManager { pub devices: [Option<WdogDevice>; 4], pub count: usize }
static WDOG_MGR: SpinLock<WdogManager> = SpinLock::new(WdogManager { devices: [const { None }; 4], count: 0 });
pub fn init() { crate::serial::write_str(b"watchdog: ready\n"); }
pub fn sys_watchdog_set(_timeout: u16) -> i32 { 0 }
pub fn sys_watchdog_ping() -> i32 { 0 }

/// 喂狗（watchdogd 调用）
pub fn kick() {
    let _ = sys_watchdog_ping();
}
