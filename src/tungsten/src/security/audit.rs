// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later
pub fn init() { crate::serial::write_str(b"  subsystem: init\n"); }
pub fn probe() { crate::serial::write_str(b"  subsystem: probe\n"); }

/// 审计日志刷盘（securityd 调用）
pub fn flush() {}
