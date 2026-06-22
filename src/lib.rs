// lib.rs — Tungsten 内核库根，导出所有子系统模块并注册全局分配器
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(function_casts_as_integer)]

extern crate alloc;

/// 内核全局分配器 — 将 alloc crate 的分配请求路由到 SLAB
#[global_allocator]
static KERNEL_ALLOC: crate::mm::slab::KernelAllocator = crate::mm::slab::KernelAllocator;

// ── 子系统模块导出 ──

pub mod bootinfo;
pub mod limine_boot;
pub mod arch;
pub mod console;
pub mod serial;
pub mod font_port;
pub mod sync;
pub mod mm;
pub mod sched;
pub mod syscall;
pub mod ipc;
pub mod devices;
pub mod fs;
pub mod uxiloader;
pub mod ai;
pub mod net;
pub mod audio;
pub mod usb;
pub mod virtio;
pub mod security;
pub mod proc;
pub mod drm;
pub mod block;
pub mod crypto;
pub mod tty;
pub mod pipe;
pub mod shm;
pub mod mq;
pub mod sem;
pub mod timer;
pub mod power;
pub mod smp;
pub mod kmod;
pub mod ptrace;
pub mod cgroup;
pub mod bpf;
pub mod kvm;
pub mod watchdog;
pub mod random;
pub mod errno;
pub mod cpu;
pub mod backtrace;
pub mod perf;
pub mod version;
pub mod service;

/// 内核版本号（与 ver.json KERNEL_VERSION 一致）
pub use version::VERSION;

/// 内核名称常量
pub const NAME: &str = "Tungsten";

/// 将 u64 格式化为十进制 ASCII 并输出到串口
pub fn serial_put_u64(val: u64) {
    let mut buf = [0u8; 20];
    if val == 0 {
        serial::write_str(b"0");
        return;
    }
    let mut n = val;
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    serial::write_str(&buf[i..]);
}

/// 将 u64 格式化为十六进制 ASCII 并输出到串口
pub fn serial_put_u64_hex(val: u64) {
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> ((15 - i) * 4)) & 0xF) as u8;
        buf[i + 2] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
    }
    serial::write_str(&buf);
}
