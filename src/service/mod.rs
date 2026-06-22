// mod.rs — Tungsten 平台服务管理器
// 平台服务 + 独立应用服务，任何程序均不得独占整机
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod app;

use crate::proc::task;
use crate::sched;
use crate::sync::SpinLock;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// 服务类别：平台基础设施 vs 用户应用
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
pub enum ServiceKind {
    Platform = 0,
    Application = 1,
}

/// 服务运行环级（与四层架构对齐）
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
pub enum ServiceRing {
    Kernel = 0,
    Driver = 1,
    IoFs = 2,
    User = 3,
}

/// 服务生命周期状态
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
pub enum ServiceState {
    Stopped = 0,
    Starting = 1,
    Running = 2,
    Error = 3,
}

/// 单条服务描述
pub struct ServiceDesc {
    pub name: &'static [u8],
    pub kind: ServiceKind,
    pub ring: ServiceRing,
    pub entry: unsafe extern "C" fn() -> !,
    pub stack: usize,
    pub uid: u32,
    pub gid: u32,
    pub state: ServiceState,
    pub tid: u64,
    pub heartbeat: AtomicU64,
}

impl ServiceDesc {
    /// 构造未启动的服务项
    const fn new(
        name: &'static [u8],
        kind: ServiceKind,
        ring: ServiceRing,
        entry: unsafe extern "C" fn() -> !,
        stack: usize,
        uid: u32,
        gid: u32,
    ) -> Self {
        ServiceDesc {
            name,
            kind,
            ring,
            entry,
            stack,
            uid,
            gid,
            state: ServiceState::Stopped,
            tid: 0,
            heartbeat: AtomicU64::new(0),
        }
    }
}

const MAX_SERVICES: usize = 24;

struct ServiceRegistry {
    table: [Option<ServiceDesc>; MAX_SERVICES],
    count: usize,
}

static REGISTRY: SpinLock<ServiceRegistry> = SpinLock::new(ServiceRegistry {
    table: [const { None }; MAX_SERVICES],
    count: 0,
});

static OS_LAYER_ENTRY: SpinLock<Option<extern "C" fn()>> = SpinLock::new(None);
static SERVICE_TICK: AtomicU32 = AtomicU32::new(0);
static VFS_RING2_INIT: AtomicBool = AtomicBool::new(false);

/// 服务心跳（由各服务主循环调用）
pub(crate) fn beat() {
    SERVICE_TICK.fetch_add(1, Ordering::Relaxed);
}

/// 注册 OS 层入口（Ada TungstenOS，Ring 2 服务线程执行）
pub fn set_os_layer_entry(entry: extern "C" fn()) {
    *OS_LAYER_ENTRY.lock() = Some(entry);
}

/// 初始化服务表（仅平台基础设施，应用由 appd 独立拉起）
pub fn init() {
    app::init();
    let mut reg = REGISTRY.lock();
    reg.count = 0;
    register_svc(&mut reg, ServiceDesc::new(b"vfsd", ServiceKind::Platform, ServiceRing::IoFs, service_vfsd, 32768, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"blockd", ServiceKind::Platform, ServiceRing::Driver, service_blockd, 24576, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"netd", ServiceKind::Platform, ServiceRing::Driver, service_netd, 32768, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"devmgr", ServiceKind::Platform, ServiceRing::Driver, service_devmgr, 24576, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"displayd", ServiceKind::Platform, ServiceRing::Driver, service_displayd, 24576, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"sessiond", ServiceKind::Platform, ServiceRing::IoFs, service_sessiond, 16384, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"securityd", ServiceKind::Platform, ServiceRing::Kernel, service_securityd, 16384, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"entropyd", ServiceKind::Platform, ServiceRing::Kernel, service_entropyd, 12288, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"powerd", ServiceKind::Platform, ServiceRing::Kernel, service_powerd, 12288, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"watchdogd", ServiceKind::Platform, ServiceRing::Kernel, service_watchdogd, 8192, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"timerd", ServiceKind::Platform, ServiceRing::Kernel, service_timerd, 8192, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"ipc_bus", ServiceKind::Platform, ServiceRing::IoFs, service_ipc_bus, 16384, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"appd", ServiceKind::Platform, ServiceRing::IoFs, service_appd, 16384, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"os_layer", ServiceKind::Platform, ServiceRing::IoFs, service_os_layer, 65536, 0, 0));
    register_svc(&mut reg, ServiceDesc::new(b"idle", ServiceKind::Platform, ServiceRing::Kernel, service_idle, 8192, 0, 0));
    crate::serial::write_str(b"service: ");
    crate::serial_put_u64(reg.count as u64);
    crate::serial::write_str(b" platform services registered\n");
}

/// 向注册表追加服务
fn register_svc(reg: &mut ServiceRegistry, desc: ServiceDesc) {
    if reg.count >= MAX_SERVICES {
        return;
    }
    reg.table[reg.count] = Some(desc);
    reg.count += 1;
}

/// 启动全部平台服务并进入调度器
pub fn bootstrap_platform() -> ! {
    let init_pid = crate::proc::with_proc_manager(|mgr| {
        unsafe { mgr.init_proc.as_ref().map(|p| p.pid).unwrap_or(1) }
    });

    crate::proc::with_proc_manager(|mgr| {
        let mut reg = REGISTRY.lock();
        for i in 0..reg.count {
            if let Some(svc) = reg.table[i].as_mut() {
                svc.state = ServiceState::Starting;
                if let Some(tid) = task::spawn_kthread(
                    mgr,
                    svc.name,
                    svc.entry,
                    init_pid,
                    svc.uid,
                    svc.gid,
                    svc.stack,
                ) {
                    svc.tid = tid;
                    svc.state = ServiceState::Running;
                    crate::serial::write_str(b"service: start ");
                    crate::serial::write_str(svc.name);
                    crate::serial::write_str(b" ring=");
                    crate::serial_put_u64(svc.ring as u64);
                    crate::serial::write_str(b" tid=");
                    crate::serial_put_u64(tid);
                    crate::serial::write_str(b"\n");
                } else {
                    svc.state = ServiceState::Error;
                }
            }
        }
    });

    crate::serial::write_str(b"service: platform online, entering scheduler\n");
    sched::start();
}

/// 格式化平台服务列表
pub fn format_list(buf: &mut [u8]) -> usize {
    let reg = REGISTRY.lock();
    let mut pos = 0;
    let mut line = |s: &[u8]| {
        if pos + s.len() + 1 > buf.len() {
            return;
        }
        buf[pos..pos + s.len()].copy_from_slice(s);
        pos += s.len();
        buf[pos] = b'\n';
        pos += 1;
    };
    line(b"NAME         RING TID  KIND ST");
    for i in 0..reg.count {
        if let Some(svc) = &reg.table[i] {
            let mut row = [b' '; 40];
            let n = svc.name.len().min(12);
            row[0..n].copy_from_slice(&svc.name[..n]);
            row[13] = b'0' + (svc.ring as u8);
            write_u64_field(&mut row[18..24], svc.tid);
            row[25] = if svc.kind == ServiceKind::Platform { b'P' } else { b'A' };
            row[27] = match svc.state {
                ServiceState::Running => b'R',
                ServiceState::Starting => b'S',
                ServiceState::Error => b'E',
                ServiceState::Stopped => b'-',
            };
            line(&row);
        }
    }
    pos
}

/// 格式化平台 + 应用全部服务
pub fn format_all(buf: &mut [u8]) -> usize {
    let n1 = format_list(buf);
    if n1 > 0 && n1 < buf.len() {
        buf[n1] = b'\n';
    }
    let n2 = app::format_list(&mut buf[n1.saturating_add(1)..]);
    n1 + 1 + n2
}

/// 启动 .uxi 应用服务
pub fn start_app(path: &str) -> i32 {
    app::start_uxi_path(path)
}

/// 停止应用服务
pub fn stop_app(name: &str) -> bool {
    app::stop_by_name(name)
}

/// 将数字写入固定宽度字段
fn write_u64_field(buf: &mut [u8], val: u64) {
    let mut tmp = [0u8; 20];
    let mut n = val;
    let mut len = 0usize;
    if n == 0 {
        buf[buf.len().saturating_sub(1)] = b'0';
        return;
    }
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    let start = buf.len().saturating_sub(len);
    for i in 0..len {
        if start + i < buf.len() {
            buf[start + i] = tmp[len - 1 - i];
        }
    }
}

// ── 各平台服务主循环 ──

/// VFS / Ring2 文件系统服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_vfsd() -> ! {
    if !VFS_RING2_INIT.swap(true, Ordering::AcqRel) {
        crate::fs::ring2_interface::init_ring2_fs();
    }
    loop {
        crate::fs::segment_cache::evict_lru();
        beat();
        sched::yield_now();
    }
}

/// 块设备 I/O 调度服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_blockd() -> ! {
    loop {
        crate::block::io_sched::probe();
        beat();
        sched::yield_now();
    }
}

/// 网络协议栈服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_netd() -> ! {
    loop {
        crate::net::poll();
        beat();
        sched::yield_now();
    }
}

/// 设备管理器（热插拔与输入轮询）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_devmgr() -> ! {
    loop {
        crate::devices::input::ps2::poll();
        crate::devices::input::usb::hid::poll();
        beat();
        sched::yield_now();
    }
}

/// 显示合成 / DRM 刷新服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_displayd() -> ! {
    loop {
        crate::drm::refresh();
        beat();
        sched::yield_now();
    }
}

/// 多用户会话管理服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_sessiond() -> ! {
    loop {
        crate::tty::session_tick();
        beat();
        sched::yield_now();
    }
}

/// 安全审计与 LSM 事件泵
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_securityd() -> ! {
    loop {
        crate::security::audit::flush();
        beat();
        sched::yield_now();
    }
}

/// 内核熵池搅拌服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_entropyd() -> ! {
    loop {
        crate::random::stir();
        beat();
        sched::yield_now();
    }
}

/// 电源与温控策略服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_powerd() -> ! {
    loop {
        crate::power::thermal::poll();
        crate::power::cpufreq::tick();
        beat();
        sched::yield_now();
    }
}

/// 看门狗喂狗服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_watchdogd() -> ! {
    loop {
        crate::watchdog::kick();
        beat();
        sched::yield_now();
    }
}

/// 高精度定时器维护
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_timerd() -> ! {
    loop {
        crate::timer::tick_update();
        beat();
        sched::yield_now();
    }
}

/// 跨服务 IPC 消息总线
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_ipc_bus() -> ! {
    loop {
        crate::ipc::dispatch_pending();
        beat();
        sched::yield_now();
    }
}

/// 应用服务监督器（拉起默认应用并均衡配额）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_appd() -> ! {
    static DEFAULTS_STARTED: AtomicBool = AtomicBool::new(false);
    if !DEFAULTS_STARTED.swap(true, Ordering::AcqRel) {
        crate::serial::write_str(b"appd: launching default application services\n");
        app::start_defaults();
    }
    loop {
        app::supervise();
        beat();
        sched::yield_now();
    }
}

/// TungstenOS Ada 系统层（Ring 2）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_os_layer() -> ! {
    if let Some(entry) = *OS_LAYER_ENTRY.lock() {
        crate::serial::write_str(b"service: os_layer entering Ada runtime\n");
        entry();
        crate::serial::write_str(b"service: os_layer returned\n");
    } else {
        crate::serial::write_str(b"service: os_layer idle (no module)\n");
    }
    loop {
        beat();
        sched::yield_now();
    }
}

/// 空闲服务
#[unsafe(no_mangle)]
pub unsafe extern "C" fn service_idle() -> ! {
    loop {
        beat();
        sched::yield_now();
        core::arch::asm!("sti; hlt", options(nomem, nostack));
    }
}
