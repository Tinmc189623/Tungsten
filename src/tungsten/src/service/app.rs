// app.rs — 应用服务注册与生命周期
// 所有用户态程序以独立服务运行，受配额约束，不得独占整机
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ServiceState, beat};
use crate::proc::task;
use crate::sched;
use crate::sync::SpinLock;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// 应用服务资源配额
#[derive(Clone, Copy)]
pub struct AppQuota {
    /// 内核栈上限（字节）
    pub stack_bytes: usize,
    /// 连续工作节拍上限，超出后强制让出 CPU
    pub cpu_quantum: u32,
    /// 地址空间软上限（KB）
    pub mem_kb: u64,
}

impl AppQuota {
    /// 默认交互式应用配额（shell、小工具）
    pub const INTERACTIVE: AppQuota = AppQuota {
        stack_bytes: 24 * 1024,
        cpu_quantum: 8,
        mem_kb: 16 * 1024,
    };

    /// 后台守护应用配额
    pub const DAEMON: AppQuota = AppQuota {
        stack_bytes: 16 * 1024,
        cpu_quantum: 4,
        mem_kb: 8 * 1024,
    };

    /// 系统层模块配额（Ada TungstenOS 组件）
    pub const SYSTEM_MODULE: AppQuota = AppQuota {
        stack_bytes: 48 * 1024,
        cpu_quantum: 16,
        mem_kb: 64 * 1024,
    };
}

/// 单条应用服务记录
pub struct AppService {
    pub id: u32,
    pub name: [u8; 24],
    pub name_len: u8,
    pub state: ServiceState,
    pub tid: u64,
    pub uid: u32,
    pub gid: u32,
    pub quota: AppQuota,
    pub entry: Option<unsafe extern "C" fn() -> !>,
    pub uxi_entry: u64,
    pub beats: AtomicU32,
    pub slice_used: AtomicU32,
}

impl AppService {
    /// 构造未启动的应用槽
    const fn empty() -> Self {
        AppService {
            id: 0,
            name: [0; 24],
            name_len: 0,
            state: ServiceState::Stopped,
            tid: 0,
            uid: 1000,
            gid: 1000,
            quota: AppQuota::INTERACTIVE,
            entry: None,
            uxi_entry: 0,
            beats: AtomicU32::new(0),
            slice_used: AtomicU32::new(0),
        }
    }
}

const MAX_APPS: usize = 16;

struct AppRegistry {
    slots: [AppService; MAX_APPS],
    count: usize,
    next_id: u32,
}

static APPS: SpinLock<AppRegistry> = SpinLock::new(AppRegistry {
    slots: [const { AppService::empty() }; MAX_APPS],
    count: 0,
    next_id: 1,
});

/// 启动时待绑定的应用槽索引
static BOOT_SLOT: AtomicUsize = AtomicUsize::new(usize::MAX);
static SPAWN_LOCK: SpinLock<()> = SpinLock::new(());

unsafe extern "C" {
    fn kernel_shell_task() -> !;
}

/// 初始化应用服务表（不自动启动，由 appd 拉起）
pub fn init() {
    let mut reg = APPS.lock();
    reg.count = 0;
    reg.next_id = 1;
    crate::serial::write_str(b"app: application service registry ready\n");
}

/// 启动默认应用集（shelld 等），与平台服务隔离
pub fn start_defaults() {
    let _ = start_builtin(
        b"shelld",
        kernel_shell_task,
        0,
        0,
        AppQuota::INTERACTIVE,
    );
}

/// 注册并启动内置应用服务
pub fn start_builtin(
    name: &[u8],
    entry: unsafe extern "C" fn() -> !,
    uid: u32,
    gid: u32,
    quota: AppQuota,
) -> Option<u32> {
    let slot_idx = {
        let mut reg = APPS.lock();
        if reg.count >= MAX_APPS {
            return None;
        }
        let idx = reg.count;
        let id = reg.next_id;
        reg.next_id += 1;
        let slot = &mut reg.slots[idx];
        slot.id = id;
        let n = name.len().min(24);
        slot.name[..n].copy_from_slice(&name[..n]);
        slot.name_len = n as u8;
        slot.state = ServiceState::Starting;
        slot.uid = uid;
        slot.gid = gid;
        slot.quota = quota;
        slot.entry = Some(entry);
        slot.uxi_entry = 0;
        slot.beats = AtomicU32::new(0);
        slot.slice_used = AtomicU32::new(0);
        reg.count += 1;
        idx
    };

    let tid = spawn_app_thread(slot_idx, quota.stack_bytes, uid, gid)?;
    let mut reg = APPS.lock();
    let slot = &mut reg.slots[slot_idx];
    slot.tid = tid;
    slot.state = ServiceState::Running;
    crate::serial::write_str(b"app: start ");
    crate::serial::write_str(name);
    crate::serial::write_str(b" tid=");
    crate::serial_put_u64(tid);
    crate::serial::write_str(b" (independent)\n");
    Some(slot.id)
}

/// 从 .uxi 路径加载并作为独立应用服务启动
pub fn start_uxi_path(path: &str) -> i32 {
    let path = path.trim();
    if path.is_empty() {
        return -1;
    }
    let fd = crate::fs::sys_open(path, 0);
    if fd < 0 {
        return -2;
    }
    let mut data_buf = [0u8; 65536];
    let n = crate::fs::sys_read(fd, &mut data_buf);
    crate::fs::sys_close(fd);
    if n <= 0 {
        return -3;
    }
    let data = &data_buf[..n as usize];
    let prog = unsafe { crate::uxiloader::load_uxi(data) };
    let prog = match prog {
        Some(p) => p,
        None => return -4,
    };

    let base_name = path.rsplit('/').next().unwrap_or(path);
    let name_bytes = base_name.as_bytes();
    let slot_idx = {
        let mut reg = APPS.lock();
        if reg.count >= MAX_APPS {
            return -5;
        }
        let idx = reg.count;
        let id = reg.next_id;
        reg.next_id += 1;
        let slot = &mut reg.slots[idx];
        slot.id = id;
        let nlen = name_bytes.len().min(24);
        slot.name[..nlen].copy_from_slice(&name_bytes[..nlen]);
        slot.name_len = nlen as u8;
        slot.state = ServiceState::Starting;
        slot.uid = 1000;
        slot.gid = 1000;
        slot.quota = AppQuota::DAEMON;
        slot.entry = None;
        slot.uxi_entry = prog.entry;
        slot.beats = AtomicU32::new(0);
        slot.slice_used = AtomicU32::new(0);
        reg.count += 1;
        idx
    };

    let stack = prog.stack_size.max(16 * 1024) as usize;
    let tid = match spawn_app_thread(slot_idx, stack.min(64 * 1024), 1000, 1000) {
        Some(t) => t,
        None => {
            let mut reg = APPS.lock();
            reg.count -= 1;
            return -6;
        }
    };

    let mut reg = APPS.lock();
    reg.slots[slot_idx].tid = tid;
    reg.slots[slot_idx].state = ServiceState::Running;
    crate::serial::write_str(b"app: uxi service ");
    crate::serial::write_str(name_bytes);
    crate::serial::write_str(b" tid=");
    crate::serial_put_u64(tid);
    crate::serial::write_str(b"\n");
    slot_idx as i32 + 1
}

/// 按名称停止应用服务（标记停止，线程自行退出需后续扩展）
pub fn stop_by_name(name: &str) -> bool {
    let name = name.trim();
    let mut reg = APPS.lock();
    for i in 0..reg.count {
        let slot = &mut reg.slots[i];
        let n = slot.name_len as usize;
        if n > 0 && core::str::from_utf8(&slot.name[..n]).ok() == Some(name) {
            slot.state = ServiceState::Stopped;
            sched::penalize_tid(slot.tid, 1000);
            crate::serial::write_str(b"app: stop requested for ");
            crate::serial::write_str(name.as_bytes());
            crate::serial::write_str(b"\n");
            return true;
        }
    }
    false
}

/// 监督循环：检查配额违规并均衡调度
pub fn supervise() {
    let reg = APPS.lock();
    for i in 0..reg.count {
        let slot = &reg.slots[i];
        if slot.state != ServiceState::Running {
            continue;
        }
        let used = slot.slice_used.load(Ordering::Relaxed);
        if used > slot.quota.cpu_quantum {
            sched::penalize_tid(slot.tid, used as u64);
            slot.slice_used.store(0, Ordering::Relaxed);
        }
    }
}

/// 当前应用服务协作点（shell 等长循环中调用）
pub fn cooperative_point() {
    let tid = sched::current_tid();
    if tid == 0 {
        beat();
        sched::yield_now();
        return;
    }
    let reg = APPS.lock();
    for i in 0..reg.count {
        let slot = &reg.slots[i];
        if slot.tid == tid {
            slot.beats.fetch_add(1, Ordering::Relaxed);
            let used = slot.slice_used.fetch_add(1, Ordering::Relaxed) + 1;
            if used >= slot.quota.cpu_quantum {
                drop(reg);
                sched::penalize_current(used as u64);
                sched::yield_now();
                let reg2 = APPS.lock();
                if i < reg2.count {
                    reg2.slots[i].slice_used.store(0, Ordering::Relaxed);
                }
                beat();
                return;
            }
            break;
        }
    }
    drop(reg);
    beat();
    sched::yield_now();
}

/// 格式化应用服务列表
pub fn format_list(buf: &mut [u8]) -> usize {
    let reg = APPS.lock();
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
    line(b"APP          TID  UID  ST  QUOTA");
    for i in 0..reg.count {
        let slot = &reg.slots[i];
        let mut row = [b' '; 48];
        let n = slot.name_len as usize;
        row[0..n].copy_from_slice(&slot.name[..n]);
        write_u64_field(&mut row[14..20], slot.tid);
        write_u64_field(&mut row[21..26], slot.uid as u64);
        row[27] = match slot.state {
            ServiceState::Running => b'R',
            ServiceState::Starting => b'S',
            ServiceState::Stopped => b'-',
            ServiceState::Error => b'E',
        };
        write_u64_field(&mut row[32..36], slot.quota.cpu_quantum as u64);
        line(&row);
    }
    pos
}

/// 运行中的应用数量
pub fn running_count() -> usize {
    let reg = APPS.lock();
    reg.slots[..reg.count]
        .iter()
        .filter(|s| s.state == ServiceState::Running)
        .count()
}

/// 应用服务统一宿主线程
#[unsafe(no_mangle)]
pub unsafe extern "C" fn app_service_host() -> ! {
    let idx = BOOT_SLOT.load(Ordering::Acquire);
    if idx < MAX_APPS {
        let reg = APPS.lock();
        if idx < reg.count {
            let slot = &reg.slots[idx];
            if let Some(entry) = slot.entry {
                drop(reg);
                entry();
            } else if slot.uxi_entry != 0 {
                let entry_fn: extern "C" fn() = core::mem::transmute(slot.uxi_entry as *const ());
                drop(reg);
                entry_fn();
            }
        }
    }
    loop {
        cooperative_point();
    }
}

/// 在进程表中创建应用线程
fn spawn_app_thread(
    slot_idx: usize,
    stack_bytes: usize,
    uid: u32,
    gid: u32,
) -> Option<u64> {
    let _guard = SPAWN_LOCK.lock();
    BOOT_SLOT.store(slot_idx, Ordering::Release);

    let (name_buf, name_len, mem_kb) = {
        let reg = APPS.lock();
        let slot = &reg.slots[slot_idx];
        let mut name = [0u8; 24];
        let n = slot.name_len as usize;
        name[..n].copy_from_slice(&slot.name[..n]);
        (name, n, slot.quota.mem_kb)
    };

    let init_pid = crate::proc::with_proc_manager(|mgr| unsafe {
        mgr.init_proc.as_ref().map(|p| p.pid).unwrap_or(1)
    });

    crate::proc::with_proc_manager(|mgr| {
        task::spawn_kthread_with_quota(
            mgr,
            &name_buf[..name_len],
            app_service_host,
            init_pid,
            uid,
            gid,
            stack_bytes,
            mem_kb,
        )
    })
}

/// 将数字写入固定宽度字段
fn write_u64_field(buf: &mut [u8], val: u64) {
    let mut tmp = [0u8; 20];
    let mut n = val;
    let mut len = 0usize;
    if n == 0 {
        if !buf.is_empty() {
            buf[buf.len() - 1] = b'0';
        }
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
