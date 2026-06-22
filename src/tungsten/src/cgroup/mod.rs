// cgroup/mod.rs — Control Groups v2 (cgroup2)
// CPU/Memory/IO/PID 控制器
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

pub mod cpu; pub mod memory; pub mod io; pub mod pid;
use crate::sync::SpinLock;
pub struct CgroupManager { pub enabled: bool, pub hierarchy_count: u32 }
static CGROUP_MGR: SpinLock<CgroupManager> = SpinLock::new(CgroupManager { enabled: false, hierarchy_count: 0 });
pub fn init() { cpu::init(); memory::init(); io::init(); pid::init();
    crate::serial::write_str(b"cgroup: ready\n"); }
