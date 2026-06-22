// perf.rs — 性能计数器 (PMU: Performance Monitoring Unit)
// Intel PMU (IA32_PERFEVTSELx / IA32_PMCx), AMD IBS, perf_event 系统调用
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later


pub mod pmu; pub mod events; pub mod sampling;
use crate::sync::SpinLock;

pub const PERF_TYPE_HARDWARE: u32 = 0;
pub const PERF_TYPE_SOFTWARE: u32 = 1;
pub const PERF_TYPE_TRACEPOINT: u32 = 2;
pub const PERF_TYPE_HW_CACHE: u32 = 3;
pub const PERF_TYPE_RAW: u32 = 4;
pub const PERF_TYPE_BREAKPOINT: u32 = 5;

pub const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
pub const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
pub const PERF_COUNT_HW_CACHE_REFERENCES: u64 = 2;
pub const PERF_COUNT_HW_CACHE_MISSES: u64 = 3;
pub const PERF_COUNT_HW_BRANCH_INSTRUCTIONS: u64 = 4;
pub const PERF_COUNT_HW_BRANCH_MISSES: u64 = 5;
pub const PERF_COUNT_HW_BUS_CYCLES: u64 = 6;
pub const PERF_COUNT_HW_STALLED_CYCLES_FRONTEND: u64 = 7;
pub const PERF_COUNT_HW_STALLED_CYCLES_BACKEND: u64 = 8;
pub const PERF_COUNT_HW_REF_CPU_CYCLES: u64 = 9;

pub const PERF_COUNT_SW_CPU_CLOCK: u64 = 0;
pub const PERF_COUNT_SW_TASK_CLOCK: u64 = 1;
pub const PERF_COUNT_SW_PAGE_FAULTS: u64 = 2;
pub const PERF_COUNT_SW_CONTEXT_SWITCHES: u64 = 3;
pub const PERF_COUNT_SW_CPU_MIGRATIONS: u64 = 4;
pub const PERF_COUNT_SW_PAGE_FAULTS_MIN: u64 = 5;
pub const PERF_COUNT_SW_PAGE_FAULTS_MAJ: u64 = 6;
pub const PERF_COUNT_SW_ALIGNMENT_FAULTS: u64 = 7;
pub const PERF_COUNT_SW_EMULATION_FAULTS: u64 = 8;
pub const PERF_COUNT_SW_DUMMY: u64 = 9;

pub const PERF_FLAG_FD_NO_GROUP: u32 = 1;
pub const PERF_FLAG_FD_OUTPUT: u32 = 2;
pub const PERF_FLAG_PID_CGROUP: u32 = 4;

pub struct PerfEvent {
    pub event_type: u32, pub config: u64,
    pub sample_period: u64, pub sample_type: u64,
    pub read_format: u64, pub disabled: bool,
    pub inherit: bool, pub pinned: bool, pub exclusive: bool,
    pub exclude_user: bool, pub exclude_kernel: bool,
    pub exclude_hv: bool, pub exclude_idle: bool,
    pub mmap: bool, pub comm: bool, pub freq: bool,
    pub inherit_stat: bool, pub enable_on_exec: bool,
    pub task: bool, pub watermark: bool,
    pub precise_ip: u64, pub mmap_data: bool,
    pub sample_id_all: bool, pub exclude_host: bool,
    pub exclude_guest: bool,
}

pub struct PerfCounter {
    pub value: u64, pub enabled: bool,
    pub running: bool, pub event: PerfEvent,
    pub owner_pid: u64, pub cpu: u32,
}

pub struct PerfManager {
    pub counters: [Option<PerfCounter>; 64],
    pub count: usize, pub pmu_version: u8,
    pub num_general_counters: u8,
    pub num_fixed_counters: u8,
}

static PERF_MGR: SpinLock<PerfManager> = SpinLock::new(PerfManager {
    counters: [const { None }; 64], count: 0,
    pmu_version: 0, num_general_counters: 0, num_fixed_counters: 0,
});

pub fn init() {
    pmu::detect(); events::init();
    crate::serial::write_str(b"perf: PMU initialized\n");
}

pub fn sys_perf_event_open(_attr: *const PerfEvent, _pid: i32, _cpu: i32, _group_fd: i32, _flags: u64) -> i32 { -1 }
pub fn read_counter(_fd: i32) -> u64 { 0 }
