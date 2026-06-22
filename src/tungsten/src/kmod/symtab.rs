// kmod/symtab.rs — 内核模块符号表
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;

const SYM_MAX: usize = 256;

/// 内核符号
#[derive(Clone, Copy)]
pub struct KernelSymbol {
    pub name: [u8; 48],
    pub name_len: usize,
    pub addr: u64,
}

/// 符号表
pub struct SymTab {
    syms: [KernelSymbol; SYM_MAX],
    count: usize,
}

impl SymTab {
    pub const fn new() -> Self {
        SymTab {
            syms: [KernelSymbol { name: [0; 48], name_len: 0, addr: 0 }; SYM_MAX],
            count: 0,
        }
    }

    /// 注册符号
    pub fn register(&mut self, name: &[u8], addr: u64) -> bool {
        if self.count >= SYM_MAX {
            return false;
        }
        let nlen = name.len().min(47);
        let s = &mut self.syms[self.count];
        s.name[..nlen].copy_from_slice(&name[..nlen]);
        s.name_len = nlen;
        s.addr = addr;
        self.count += 1;
        true
    }

    /// 按名称查找符号
    pub fn lookup(&self, name: &[u8]) -> Option<u64> {
        for i in 0..self.count {
            let s = &self.syms[i];
            if s.name_len == name.len() && &s.name[..s.name_len] == name {
                return Some(s.addr);
            }
        }
        None
    }

    /// 列出符号到缓冲区
    pub fn list(&self, buf: &mut [u8]) -> usize {
        let mut pos = 0usize;
        for i in 0..self.count {
            let s = &self.syms[i];
            let line = &s.name[..s.name_len];
            if pos + line.len() + 2 > buf.len() {
                break;
            }
            buf[pos..pos + line.len()].copy_from_slice(line);
            pos += line.len();
            buf[pos] = b'\n';
            pos += 1;
        }
        pos
    }
}

static SYMTAB: SpinLock<SymTab> = SpinLock::new(SymTab::new());

/// 初始化符号表并注册核心符号
pub fn init() {
    let mut tab = SYMTAB.lock();
    tab.register(b"tungsten_init", crate::sched::init as usize as u64);
    tab.register(b"tungsten_syscall", crate::syscall::dispatch as usize as u64);
    crate::serial::write_str(b"  kmod: symtab ready\n");
}

/// 注册符号
pub fn register(name: &[u8], addr: u64) -> bool {
    SYMTAB.lock().register(name, addr)
}

/// 查找符号地址
pub fn lookup(name: &[u8]) -> Option<u64> {
    SYMTAB.lock().lookup(name)
}

/// 列出已注册符号
pub fn list(buf: &mut [u8]) -> usize {
    SYMTAB.lock().list(buf)
}

pub fn probe() {}
