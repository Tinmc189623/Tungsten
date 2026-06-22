// kmod/verify.rs — 内核模块签名与格式校验
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::proc::elf;

/// 校验模块镜像（ELF 或 Tungsten 模块魔数）
pub fn verify_module(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    if elf::validate(data) {
        return true;
    }
    // Tungsten 模块魔数 "TMOD"
    data[0..4] == *b"TMOD"
}

pub fn init() {
    crate::serial::write_str(b"  kmod: verify ready\n");
}

pub fn probe() {}
