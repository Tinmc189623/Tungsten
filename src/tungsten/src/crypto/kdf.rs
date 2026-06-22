// kdf.rs — 密钥派生 (HKDF-SHA256, RFC 5869)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::hmac;

/// HKDF-Extract: PRK = HMAC(salt, IKM)
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let salt_key = if salt.is_empty() { &[0u8; 32][..] } else { salt };
    hmac::hmac_sha256(salt_key, ikm)
}

/// HKDF-Expand: 输出任意长度密钥材料 (最多 255 * 32 字节)
pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], len: usize) -> [u8; 64] {
    let out_len = core::cmp::min(len, 64);
    let mut okm = [0u8; 64];
    let mut t = [0u8; 32];
    let mut t_len = 0usize;
    let mut pos = 0usize;
    let mut counter = 1u8;

    while pos < out_len {
        let mut input = [0u8; 32 + 256 + 1];
        let mut input_len = 0usize;

        if t_len > 0 {
            input[..t_len].copy_from_slice(&t[..t_len]);
            input_len = t_len;
        }
        let info_take = info.len().min(256);
        input[input_len..input_len + info_take].copy_from_slice(&info[..info_take]);
        input_len += info_take;
        input[input_len] = counter;
        input_len += 1;

        t = hmac::hmac_sha256(prk, &input[..input_len]);
        t_len = 32;

        let copy = core::cmp::min(32, out_len - pos);
        okm[pos..pos + copy].copy_from_slice(&t[..copy]);
        pos += copy;
        counter = counter.wrapping_add(1);
    }

    okm
}

/// HKDF-SHA256 一步派生 (salt + IKM + info → 输出密钥)
pub fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> [u8; 64] {
    let prk = hkdf_extract(salt, ikm);
    hkdf_expand(&prk, info, len)
}

/// 子系统初始化
pub fn init() {
    crate::serial::write_str(b"  crypto-kdf: ready\n");
}

/// 子系统探测
pub fn probe() {}
