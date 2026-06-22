// bpf/mod.rs — eBPF 虚拟机 (Berkeley Packet Filter)
// eBPF 字节码 JIT (x86_64), maps, helpers, verifier
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

pub mod jit; pub mod verifier; pub mod maps; pub mod helpers;
use crate::sync::SpinLock;
pub struct BpfManager { pub programs_loaded: u32, pub maps_created: u32 }
static BPF_MGR: SpinLock<BpfManager> = SpinLock::new(BpfManager { programs_loaded: 0, maps_created: 0 });
pub fn init() { crate::serial::write_str(b"bpf: ready\n"); }
pub fn sys_bpf(_cmd: i32, _attr: *mut (), _size: u32) -> i32 { -1 }
