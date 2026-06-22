// kmod/loader.rs — 可加载内核模块加载器
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{KmodInfo, KMOD_MGR};
use crate::proc::elf;

const KMOD_LOAD_BASE: u64 = 0xFFFF_C000_0000_0000;

/// 从内存镜像加载 ELF 模块
pub unsafe fn load_image(name: &str, data: &[u8]) -> i32 {
    if !super::verify::verify_module(data) {
        return -22;
    }
    let loaded = match elf::load_segments(data, KMOD_LOAD_BASE) {
        Some(l) => l,
        None => return -2,
    };

    let mut mgr = KMOD_MGR.lock();
    if mgr.count >= mgr.modules.len() {
        return -12;
    }

    let mut info = KmodInfo {
        name: [0; 32],
        version: 1,
        base: loaded.base,
        size: loaded.size,
        refcount: 1,
        state: 1,
    };
    let nlen = name.len().min(31);
    info.name[..nlen].copy_from_slice(&name.as_bytes()[..nlen]);
    let idx = mgr.count;
    mgr.modules[idx] = Some(info);
    mgr.count += 1;

    super::symtab::register(&format_name(name)[..4 + name.len().min(43)], loaded.entry);
    crate::serial::write_str(b"kmod: loaded ");
    crate::serial::write_str(name.as_bytes());
    crate::serial::write_str(b"\n");
    0
}

fn format_name(name: &str) -> [u8; 48] {
    let mut out = [0u8; 48];
    let prefix = b"mod_";
    out[..4].copy_from_slice(prefix);
    let nlen = name.len().min(43);
    out[4..4 + nlen].copy_from_slice(&name.as_bytes()[..nlen]);
    out
}

/// 按名称从 VFS 加载模块
pub fn load_by_name(name: &str) -> i32 {
    let mut path = [0u8; 128];
    let prefix = b"/Applications/Drivers/";
    let suffix = b".uxi";
    let nlen = name.len().min(64);
    let total = prefix.len() + nlen + suffix.len();
    if total >= path.len() {
        return -36;
    }
    path[..prefix.len()].copy_from_slice(prefix);
    path[prefix.len()..prefix.len() + nlen].copy_from_slice(&name.as_bytes()[..nlen]);
    path[prefix.len() + nlen..total].copy_from_slice(suffix);

    let path_str = core::str::from_utf8(&path[..total]).unwrap_or("");
    let fd = crate::fs::sys_open(path_str, 0);
    if fd < 0 {
        return fd as i32;
    }

    let mut buf = [0u8; 65536];
    let n = crate::fs::sys_read(fd, &mut buf);
    crate::fs::sys_close(fd);
    if n <= 0 {
        return -2;
    }
    unsafe { load_image(name, &buf[..n as usize]) }
}

/// 列出已加载模块到缓冲区
pub fn list_loaded(buf: &mut [u8]) -> usize {
    let mgr = KMOD_MGR.lock();
    let mut pos = 0usize;
    for i in 0..mgr.count {
        if let Some(ref m) = mgr.modules[i] {
            let name = &m.name;
            let end = name.iter().position(|&b| b == 0).unwrap_or(32);
            let line = &name[..end];
            if pos + line.len() + 2 > buf.len() {
                break;
            }
            buf[pos..pos + line.len()].copy_from_slice(line);
            pos += line.len();
            buf[pos] = b'\n';
            pos += 1;
        }
    }
    pos
}

pub fn init() {
    crate::serial::write_str(b"  kmod: loader ready\n");
}

pub fn probe() {}
