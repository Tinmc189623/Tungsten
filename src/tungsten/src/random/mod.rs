// random/mod.rs — 随机数生成器 (/dev/random, /dev/urandom)
// RDRAND/RDSEED 硬件随机、ChaCha20 DRBG、熵收集
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub struct EntropyPool { pub data: [u8; 64], pub pos: usize, pub entropy_count: u32 }
pub struct RandomManager { pub pool: EntropyPool, pub rdrand_available: bool, pub rdseed_available: bool }
static RANDOM_MGR: SpinLock<RandomManager> = SpinLock::new(RandomManager {
    pool: EntropyPool { data: [0; 64], pos: 0, entropy_count: 0 },
    rdrand_available: false, rdseed_available: false,
});
pub fn init() { crate::serial::write_str(b"random: entropy pool initialized\n"); }
pub fn sys_getrandom(_buf: *mut u8, _len: usize, _flags: u32) -> isize { -1 }

/// 搅拌熵池（entropyd 调用）
pub fn stir() {
    let mut mgr = RANDOM_MGR.lock();
    let t = crate::sched::ticks() as u8;
    let pos = mgr.pool.pos % 64;
    mgr.pool.data[pos] ^= t;
    mgr.pool.pos = mgr.pool.pos.wrapping_add(1);
    mgr.pool.entropy_count = mgr.pool.entropy_count.saturating_add(1);
}
