// kmod/mod.rs — 可加载内核模块 (LKLM)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod depmod;
pub mod loader;
pub mod symtab;
pub mod verify;

use crate::sync::SpinLock;

/// 已加载模块信息
pub struct KmodInfo {
    pub name: [u8; 32],
    pub version: u32,
    pub base: u64,
    pub size: u64,
    pub refcount: u32,
    pub state: u8,
}

/// 模块管理器
pub struct KmodManager {
    pub modules: [Option<KmodInfo>; 64],
    pub count: usize,
}

static KMOD_MGR: SpinLock<KmodManager> = SpinLock::new(KmodManager {
    modules: [const { None }; 64],
    count: 0,
});

/// 初始化内核模块子系统
pub fn init() {
    symtab::init();
    loader::init();
    verify::init();
    crate::serial::write_str(b"kmod: subsystem ready\n");
}

/// 按名称加载模块
pub fn load(name: &str) -> i32 {
    loader::load_by_name(name)
}

/// 列出已加载模块
pub fn list(buf: &mut [u8]) -> usize {
    loader::list_loaded(buf)
}

/// init_module 系统调用
pub fn sys_init_module(_image: *const u8, _len: usize, _args: *const u8) -> i32 {
    -38
}

/// delete_module 系统调用
pub fn sys_delete_module(_name: *const u8, _flags: i32) -> i32 {
    -38
}
