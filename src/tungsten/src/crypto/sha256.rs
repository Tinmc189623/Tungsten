// sha256.rs — SHA-256 摘要 (FIPS 180-4)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/// SHA-256 初始哈希值
const H_INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 轮常量
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 右旋转 32 位
#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

/// 处理单个 512 位块
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        let j = i * 4;
        w[i] = u32::from_be_bytes([block[j], block[j + 1], block[j + 2], block[j + 3]]);
    }
    for i in 16..64 {
        let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
        state[0], state[1], state[2], state[3], state[4], state[5], state[6], state[7],
    );

    for i in 0..64 {
        let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// 增量 SHA-256 上下文
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buflen: usize,
    total_bytes: u64,
}

impl Sha256 {
    /// 创建新上下文
    pub const fn new() -> Self {
        Sha256 {
            state: H_INIT,
            buffer: [0u8; 64],
            buflen: 0,
            total_bytes: 0,
        }
    }

    /// 追加数据块
    pub fn update(&mut self, data: &[u8]) {
        let mut offset = 0usize;
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);

        while offset < data.len() {
            let take = core::cmp::min(64 - self.buflen, data.len() - offset);
            self.buffer[self.buflen..self.buflen + take]
                .copy_from_slice(&data[offset..offset + take]);
            self.buflen += take;
            offset += take;

            if self.buflen == 64 {
                compress(&mut self.state, &self.buffer);
                self.buflen = 0;
            }
        }
    }

    /// 完成摘要计算
    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        let rem = self.buflen;

        self.buffer[rem] = 0x80;
        if rem >= 56 {
            self.buffer[rem + 1..].fill(0);
            compress(&mut self.state, &self.buffer);
            self.buffer.fill(0);
        } else {
            self.buffer[rem + 1..56].fill(0);
        }

        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        compress(&mut self.state, &self.buffer);

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

/// 计算任意长度数据的 SHA-256 摘要
pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut ctx = Sha256::new();
    ctx.update(data);
    ctx.finish()
}

/// 子系统初始化
pub fn init() {
    crate::serial::write_str(b"  crypto-sha256: ready\n");
}

/// 子系统探测
pub fn probe() {}
