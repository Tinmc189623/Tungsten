// net/udp.rs — UDP 用户数据报协议 (RFC 768)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later
#[repr(C, packed)]
pub struct UdpHeader { pub src_port: u16, pub dst_port: u16, pub length: u16, pub checksum: u16 }
pub fn init() {}
