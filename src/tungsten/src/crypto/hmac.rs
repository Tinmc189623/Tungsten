// hmac.rs — HMAC-SHA256 (RFC 2104 / FIPS 198-1)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::sha256::{self, Sha256};

const BLOCK_SIZE: usize = 64;

/// 计算 HMAC-SHA256
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK_SIZE];

    if key.len() > BLOCK_SIZE {
        let hashed = sha256::digest(key);
        k[..32].copy_from_slice(&hashed);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finish();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finish()
}

/// 子系统初始化
pub fn init() {
    crate::serial::write_str(b"  crypto-hmac: ready\n");
}

/// 子系统探测
pub fn probe() {}
