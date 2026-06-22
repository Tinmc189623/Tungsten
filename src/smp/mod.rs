// smp/mod.rs — 对称多处理 (SMP) 初始化
// AP 枚举、per-CPU 数据结构、x2APIC IPI 核间中断
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::arch::x86_64::acpi;
use crate::arch::x86_64::apic;
use crate::sync::SpinLock;

/// 最大 CPU 数量
pub const MAX_CPUS: usize = 256;

/// 单颗逻辑 CPU 描述
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CpuInfo {
    pub apic_id: u32,
    pub enabled: bool,
    pub online: bool,
    pub is_bsp: bool,
}

/// SMP 管理器
pub struct SmpManager {
    pub cpus: [CpuInfo; MAX_CPUS],
    pub cpu_count: u32,
    pub bsp_id: u32,
}

unsafe impl Send for SmpManager {}

static SMP_MGR: SpinLock<SmpManager> = SpinLock::new(SmpManager {
    cpus: [CpuInfo {
        apic_id: 0,
        enabled: false,
        online: false,
        is_bsp: false,
    }; MAX_CPUS],
    cpu_count: 0,
    bsp_id: 0,
});

/// 初始化 SMP：从 ACPI MADT 枚举 CPU 并标记 BSP
pub fn init() {
    let bsp = apic::lapic_id();
    let mut ids = [0u32; MAX_CPUS];
    let n = unsafe { acpi::enumerate_cpus(&mut ids) };
    let mut mgr = SMP_MGR.lock();
    mgr.bsp_id = bsp;
    mgr.cpu_count = n.max(1) as u32;
    for i in 0..mgr.cpu_count as usize {
        mgr.cpus[i].apic_id = ids[i];
        mgr.cpus[i].enabled = true;
        mgr.cpus[i].online = ids[i] == bsp;
        mgr.cpus[i].is_bsp = ids[i] == bsp;
    }
    crate::serial::write_str(b"smp: ");
    crate::serial_put_u64(mgr.cpu_count as u64);
    crate::serial::write_str(b" CPUs enumerated, BSP apic_id=");
    crate::serial_put_u64(bsp as u64);
    crate::serial::write_str(b"\n");
}

/// 已枚举 CPU 数量
pub fn cpu_count() -> u32 {
    SMP_MGR.lock().cpu_count
}

/// 按索引获取 CPU 信息
pub fn cpu_info(idx: usize) -> Option<CpuInfo> {
    let mgr = SMP_MGR.lock();
    if idx < mgr.cpu_count as usize {
        Some(mgr.cpus[idx])
    } else {
        None
    }
}

/// 启动应用处理器（INIT-SIPI-SIPI，当前仅记录并发送 INIT IPI）
pub fn boot_ap(apic_id: u32) {
    crate::serial::write_str(b"smp: INIT IPI to apic_id=");
    crate::serial_put_u64(apic_id as u64);
    crate::serial::write_str(b"\n");
    apic::send_ipi(apic_id, 0xF0);
}

/// 向指定 CPU 发送 IPI
pub fn send_ipi(cpu_idx: u32, vector: u8) {
    let mgr = SMP_MGR.lock();
    if (cpu_idx as usize) < mgr.cpu_count as usize {
        apic::send_ipi(mgr.cpus[cpu_idx as usize].apic_id, vector);
    }
}

/// 列出 CPU 到缓冲区
pub fn list_cpus(buf: &mut [u8]) -> usize {
    let mgr = SMP_MGR.lock();
    let mut pos = 0usize;
    for i in 0..mgr.cpu_count as usize {
        let c = &mgr.cpus[i];
        if pos + 48 > buf.len() {
            break;
        }
        let prefix = if c.is_bsp { b"bsp " } else { b"ap  " };
        buf[pos..pos + prefix.len()].copy_from_slice(prefix);
        pos += prefix.len();
        let line = b"apic_id=";
        buf[pos..pos + line.len()].copy_from_slice(line);
        pos += line.len();
        let mut num = [0u8; 12];
        let mut v = c.apic_id as u64;
        let mut j = 12;
        if v == 0 {
            num[j - 1] = b'0';
            j -= 1;
        } else {
            while v > 0 {
                j -= 1;
                num[j] = b'0' + (v % 10) as u8;
                v /= 10;
            }
        }
        let digits = &num[j..];
        buf[pos..pos + digits.len()].copy_from_slice(digits);
        pos += digits.len();
        buf[pos] = if c.online { b'O' } else { b'-' };
        pos += 1;
        buf[pos] = b'\n';
        pos += 1;
    }
    pos
}
