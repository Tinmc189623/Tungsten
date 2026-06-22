// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later
pub fn init() { crate::serial::write_str(b"  subsystem: init\n"); }
pub fn probe() { crate::serial::write_str(b"  subsystem: probe\n"); }

/// 温控采样（powerd 调用）
pub fn poll() {}
