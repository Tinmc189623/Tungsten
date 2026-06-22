// power/mod.rs — 电源管理 (ACPI S3/S4/S5, CPU 频率调节, 热管理)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod acpi_pm;
pub mod cpufreq;
pub mod reboot;
pub mod suspend;
pub mod thermal;

use crate::sync::SpinLock;

/// 电源管理器状态
pub struct PowerManager {
    pub acpi_supported: bool,
    pub s3_available: bool,
    pub s4_available: bool,
}

static POWER_MGR: SpinLock<PowerManager> = SpinLock::new(PowerManager {
    acpi_supported: false,
    s3_available: false,
    s4_available: false,
});

/// 初始化电源子系统
pub fn init() {
    acpi_pm::probe();
    cpufreq::init();
    thermal::init();
    let mut mgr = POWER_MGR.lock();
    mgr.acpi_supported = acpi_pm::acpi_available();
    crate::serial::write_str(b"power: subsystem ready\n");
}

/// 系统重启
pub fn sys_reboot(_cmd: i32) -> ! {
    reboot::cold_reboot()
}

/// 系统关机
pub fn sys_poweroff() -> ! {
    reboot::power_off()
}
