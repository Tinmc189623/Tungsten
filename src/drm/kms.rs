// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later
pub fn init() { crate::serial::write_str(b"  subsystem: init\n"); }
pub fn probe() { crate::serial::write_str(b"  subsystem: probe\n"); }

/// KMS 扫描输出（displayd 周期调用）
pub fn scanout() {}
