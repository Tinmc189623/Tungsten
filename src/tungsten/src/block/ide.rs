// block/ide.rs — 传统 IDE 控制器（仅枚举，现代平台由 AHCI/NVMe 接管）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/// 探测 IDE 控制器（当前平台由 AHCI/NVMe 优先）
pub fn probe() {}

/// 初始化 IDE 子模块
pub fn init() {}
