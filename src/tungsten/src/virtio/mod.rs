// virtio/mod.rs — VirtIO 半虚拟化驱动框架
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod blk; pub mod net; pub mod gpu; pub mod input; pub mod entropy;
pub fn init() { crate::serial::write_str(b"virtio: initializing...\n"); blk::probe(); net::probe();
  gpu::probe(); input::probe(); crate::serial::write_str(b"virtio: ready\n"); }
