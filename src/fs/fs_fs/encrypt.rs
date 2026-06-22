// fs/fs_fs/encrypt.rs — 文件级透明加密 (fscrypt 风格)
// AES-256-XTS: 每文件独立密钥, 加密上下文存储在 xattr
// 页粒度加解密, 页索引用作 XTS tweak
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── 常量 ──

/// AES 块大小 (固定 128 位)
const AES_BLOCK_SIZE: usize = 16;
/// AES-256 密钥大小
const AES_256_KEY_SIZE: usize = 32;
/// XTS 需要双密钥 (key1 + key2)
const XTS_KEY_SIZE: usize = 64; // 512 bits
/// 加密上下文大小
const ENCRYPT_CTX_SIZE: usize = 64;

/// 加密算法标识
pub const ENCRYPT_ALG_AES256_XTS: u8 = 1;

/// 加密上下文 (存储在 inode.encrypt_ctx)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EncryptContext {
    pub version: u8,             // 上下文版本
    pub algorithm: u8,           // 加密算法 (1=AES-256-XTS)
    pub key_id: [u8; 8],        // 主密钥 ID
    pub salt: [u8; 16],         // 密钥派生盐值
    pub flags: u8,              // 标志
    pub _reserved: [u8; 37],
}

impl EncryptContext {
    pub const fn empty() -> Self {
        EncryptContext {
            version: 0, algorithm: 0,
            key_id: [0; 8], salt: [0; 16], flags: 0,
            _reserved: [0; 37],
        }
    }

    pub fn is_valid(&self) -> bool {
        self.version > 0 && self.algorithm == ENCRYPT_ALG_AES256_XTS
    }
}

// ── AES S-Box ──

static AES_SBOX: [u8; 256] = [
    0x63, 0x7C, 0x77, 0x7B, 0xF2, 0x6B, 0x6F, 0xC5, 0x30, 0x01, 0x67, 0x2B, 0xFE, 0xD7, 0xAB, 0x76,
    0xCA, 0x82, 0xC9, 0x7D, 0xFA, 0x59, 0x47, 0xF0, 0xAD, 0xD4, 0xA2, 0xAF, 0x9C, 0xA4, 0x72, 0xC0,
    0xB7, 0xFD, 0x93, 0x26, 0x36, 0x3F, 0xF7, 0xCC, 0x34, 0xA5, 0xE5, 0xF1, 0x71, 0xD8, 0x31, 0x15,
    0x04, 0xC7, 0x23, 0xC3, 0x18, 0x96, 0x05, 0x9A, 0x07, 0x12, 0x80, 0xE2, 0xEB, 0x27, 0xB2, 0x75,
    0x09, 0x83, 0x2C, 0x1A, 0x1B, 0x6E, 0x5A, 0xA0, 0x52, 0x3B, 0xD6, 0xB3, 0x29, 0xE3, 0x2F, 0x84,
    0x53, 0xD1, 0x00, 0xED, 0x20, 0xFC, 0xB1, 0x5B, 0x6A, 0xCB, 0xBE, 0x39, 0x4A, 0x4C, 0x58, 0xCF,
    0xD0, 0xEF, 0xAA, 0xFB, 0x43, 0x4D, 0x33, 0x85, 0x45, 0xF9, 0x02, 0x7F, 0x50, 0x3C, 0x9F, 0xA8,
    0x51, 0xA3, 0x40, 0x8F, 0x92, 0x9D, 0x38, 0xF5, 0xBC, 0xB6, 0xDA, 0x21, 0x10, 0xFF, 0xF3, 0xD2,
    0xCD, 0x0C, 0x13, 0xEC, 0x5F, 0x97, 0x44, 0x17, 0xC4, 0xA7, 0x7E, 0x3D, 0x64, 0x5D, 0x19, 0x73,
    0x60, 0x81, 0x4F, 0xDC, 0x22, 0x2A, 0x90, 0x88, 0x46, 0xEE, 0xB8, 0x14, 0xDE, 0x5E, 0x0B, 0xDB,
    0xE0, 0x32, 0x3A, 0x0A, 0x49, 0x06, 0x24, 0x5C, 0xC2, 0xD3, 0xAC, 0x62, 0x91, 0x95, 0xE4, 0x79,
    0xE7, 0xC8, 0x37, 0x6D, 0x8D, 0xD5, 0x4E, 0xA9, 0x6C, 0x56, 0xF4, 0xEA, 0x65, 0x7A, 0xAE, 0x08,
    0xBA, 0x78, 0x25, 0x2E, 0x1C, 0xA6, 0xB4, 0xC6, 0xE8, 0xDD, 0x74, 0x1F, 0x4B, 0xBD, 0x8B, 0x8A,
    0x70, 0x3E, 0xB5, 0x66, 0x48, 0x03, 0xF6, 0x0E, 0x61, 0x35, 0x57, 0xB9, 0x86, 0xC1, 0x1D, 0x9E,
    0xE1, 0xF8, 0x98, 0x11, 0x69, 0xD9, 0x8E, 0x94, 0x9B, 0x1E, 0x87, 0xE9, 0xCE, 0x55, 0x28, 0xDF,
    0x8C, 0xA1, 0x89, 0x0D, 0xBF, 0xE6, 0x42, 0x68, 0x41, 0x99, 0x2D, 0x0F, 0xB0, 0x54, 0xBB, 0x16,
];

static AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6A, 0xD5, 0x30, 0x36, 0xA5, 0x38, 0xBF, 0x40, 0xA3, 0x9E, 0x81, 0xF3, 0xD7, 0xFB,
    0x7C, 0xE3, 0x39, 0x82, 0x9B, 0x2F, 0xFF, 0x87, 0x34, 0x8E, 0x43, 0x44, 0xC4, 0xDE, 0xE9, 0xCB,
    0x54, 0x7B, 0x94, 0x32, 0xA6, 0xC2, 0x23, 0x3D, 0xEE, 0x4C, 0x95, 0x0B, 0x42, 0xFA, 0xC3, 0x4E,
    0x08, 0x2E, 0xA1, 0x66, 0x28, 0xD9, 0x24, 0xB2, 0x76, 0x5B, 0xA2, 0x49, 0x6D, 0x8B, 0xD1, 0x25,
    0x72, 0xF8, 0xF6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xD4, 0xA4, 0x5C, 0xCC, 0x5D, 0x65, 0xB6, 0x92,
    0x6C, 0x70, 0x48, 0x50, 0xFD, 0xED, 0xB9, 0xDA, 0x5E, 0x15, 0x46, 0x57, 0xA7, 0x8D, 0x9D, 0x84,
    0x90, 0xD8, 0xAB, 0x00, 0x8C, 0xBC, 0xD3, 0x0A, 0xF7, 0xE4, 0x58, 0x05, 0xB8, 0xB3, 0x45, 0x06,
    0xD0, 0x2C, 0x1E, 0x8F, 0xCA, 0x3F, 0x0F, 0x02, 0xC1, 0xAF, 0xBD, 0x03, 0x01, 0x13, 0x8A, 0x6B,
    0x3A, 0x91, 0x11, 0x41, 0x4F, 0x67, 0xDC, 0xEA, 0x97, 0xF2, 0xCF, 0xCE, 0xF0, 0xB4, 0xE6, 0x73,
    0x96, 0xAC, 0x74, 0x22, 0xE7, 0xAD, 0x35, 0x85, 0xE2, 0xF9, 0x37, 0xE8, 0x1C, 0x75, 0xDF, 0x6E,
    0x47, 0xF1, 0x1A, 0x71, 0x1D, 0x29, 0xC5, 0x89, 0x6F, 0xB7, 0x62, 0x0E, 0xAA, 0x18, 0xBE, 0x1B,
    0xFC, 0x56, 0x3E, 0x4B, 0xC6, 0xD2, 0x79, 0x20, 0x9A, 0xDB, 0xC0, 0xFE, 0x78, 0xCD, 0x5A, 0xF4,
    0x1F, 0xDD, 0xA8, 0x33, 0x88, 0x07, 0xC7, 0x31, 0xB1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xEC, 0x5F,
    0x60, 0x51, 0x7F, 0xA9, 0x19, 0xB5, 0x4A, 0x0D, 0x2D, 0xE5, 0x7A, 0x9F, 0x93, 0xC9, 0x9C, 0xEF,
    0xA0, 0xE0, 0x3B, 0x4D, 0xAE, 0x2A, 0xF5, 0xB0, 0xC8, 0xEB, 0xBB, 0x3C, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2B, 0x04, 0x7E, 0xBA, 0x77, 0xD6, 0x26, 0xE1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0C, 0x7D,
];

/// Rcon 常量 (AES-256 需要)
static AES_RCON: [u8; 11] = [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1B, 0x36];

// ── GF(2^8) 乘法 (用于 MixColumns 和 XTS tweak) ──

fn gf_mul2(x: u8) -> u8 {
    let r = (x as u16) << 1;
    if (r & 0x100) != 0 { (r ^ 0x11B) as u8 } else { r as u8 }
}

fn gf_mul3(x: u8) -> u8 { gf_mul2(x) ^ x }
fn gf_mul9(x: u8) -> u8 { gf_mul2(gf_mul2(gf_mul2(x))) ^ x }
fn gf_mul11(x: u8) -> u8 { gf_mul2(gf_mul2(gf_mul2(x)) ^ x) ^ x }
fn gf_mul13(x: u8) -> u8 { gf_mul2(gf_mul2(gf_mul2(x) ^ x)) ^ x }
fn gf_mul14(x: u8) -> u8 { gf_mul2(gf_mul2(gf_mul2(x) ^ x) ^ x) }

// ── AES 核心操作 ──

fn sub_bytes(state: &mut [u8; 16]) {
    for i in 0..16 { state[i] = AES_SBOX[state[i] as usize]; }
}

fn inv_sub_bytes(state: &mut [u8; 16]) {
    for i in 0..16 { state[i] = AES_INV_SBOX[state[i] as usize]; }
}

fn shift_rows(state: &mut [u8; 16]) {
    // Row 1: shift left 1
    let t = state[1];
    state[1] = state[5]; state[5] = state[9]; state[9] = state[13]; state[13] = t;
    // Row 2: shift left 2
    let t0 = state[2]; let t1 = state[6];
    state[2] = state[10]; state[10] = t0;
    state[6] = state[14]; state[14] = t1;
    // Row 3: shift left 3 (= right 1)
    let t = state[15];
    state[15] = state[11]; state[11] = state[7]; state[7] = state[3]; state[3] = t;
}

fn inv_shift_rows(state: &mut [u8; 16]) {
    let t = state[13];
    state[13] = state[9]; state[9] = state[5]; state[5] = state[1]; state[1] = t;
    let t0 = state[2]; let t1 = state[6];
    state[2] = state[10]; state[10] = t0;
    state[6] = state[14]; state[14] = t1;
    let t = state[3];
    state[3] = state[7]; state[7] = state[11]; state[11] = state[15]; state[15] = t;
}

fn mix_columns(state: &mut [u8; 16]) {
    for col in 0..4 {
        let i = col * 4;
        let s0 = state[i]; let s1 = state[i+1]; let s2 = state[i+2]; let s3 = state[i+3];
        state[i]   = gf_mul2(s0) ^ gf_mul3(s1) ^ s2 ^ s3;
        state[i+1] = s0 ^ gf_mul2(s1) ^ gf_mul3(s2) ^ s3;
        state[i+2] = s0 ^ s1 ^ gf_mul2(s2) ^ gf_mul3(s3);
        state[i+3] = gf_mul3(s0) ^ s1 ^ s2 ^ gf_mul2(s3);
    }
}

fn inv_mix_columns(state: &mut [u8; 16]) {
    for col in 0..4 {
        let i = col * 4;
        let s0 = state[i]; let s1 = state[i+1]; let s2 = state[i+2]; let s3 = state[i+3];
        state[i]   = gf_mul14(s0) ^ gf_mul11(s1) ^ gf_mul13(s2) ^ gf_mul9(s3);
        state[i+1] = gf_mul9(s0)  ^ gf_mul14(s1) ^ gf_mul11(s2) ^ gf_mul13(s3);
        state[i+2] = gf_mul13(s0) ^ gf_mul9(s1)  ^ gf_mul14(s2) ^ gf_mul11(s3);
        state[i+3] = gf_mul11(s0) ^ gf_mul13(s1) ^ gf_mul9(s2)  ^ gf_mul14(s3);
    }
}

fn add_round_key(state: &mut [u8; 16], round_key: &[u8; 16]) {
    for i in 0..16 { state[i] ^= round_key[i]; }
}

// ── AES-256 密钥扩展 ──

/// AES-256 轮密钥 (15 轮 × 16 字节 = 240 字节)
pub struct Aes256Key {
    round_keys: [u8; 240],  // 15 轮密钥 (Nk=8, Nr=14)
}

impl Aes256Key {
    /// 从 32 字节原始密钥生成轮密钥
    pub fn from_key(key: &[u8; AES_256_KEY_SIZE]) -> Self {
        let mut rk = [0u8; 240];
        // 复制初始密钥
        rk[0..32].copy_from_slice(key);

        let nk = 8;  // AES-256: 8 words
        let nr = 14; // AES-256: 14 rounds
        let mut i = nk;
        let mut rcon_idx = 1;

        while i < 4 * (nr + 1) {
            let mut temp = [rk[(i-1)*4], rk[(i-1)*4+1], rk[(i-1)*4+2], rk[(i-1)*4+3]];

            if i % nk == 0 {
                // RotWord + SubWord + Rcon
                let t = temp[0];
                temp[0] = AES_SBOX[temp[1] as usize] ^ AES_RCON[rcon_idx];
                temp[1] = AES_SBOX[temp[2] as usize];
                temp[2] = AES_SBOX[temp[3] as usize];
                temp[3] = AES_SBOX[t as usize];
                rcon_idx += 1;
            } else if nk > 6 && i % nk == 4 {
                // 仅 SubWord
                temp[0] = AES_SBOX[temp[0] as usize];
                temp[1] = AES_SBOX[temp[1] as usize];
                temp[2] = AES_SBOX[temp[2] as usize];
                temp[3] = AES_SBOX[temp[3] as usize];
            }

            let prev = (i - nk) * 4;
            rk[i*4]   = rk[prev]   ^ temp[0];
            rk[i*4+1] = rk[prev+1] ^ temp[1];
            rk[i*4+2] = rk[prev+2] ^ temp[2];
            rk[i*4+3] = rk[prev+3] ^ temp[3];
            i += 1;
        }

        Aes256Key { round_keys: rk }
    }

    /// 加密单个 128 位块
    pub fn encrypt_block(&self, block: &mut [u8; AES_BLOCK_SIZE]) {
        add_round_key(block, self.round_key(0));

        for round in 1..14 {
            sub_bytes(block);
            shift_rows(block);
            mix_columns(block);
            add_round_key(block, self.round_key(round));
        }

        // 最后一轮无 MixColumns
        sub_bytes(block);
        shift_rows(block);
        add_round_key(block, self.round_key(14));
    }

    /// 解密单个 128 位块
    pub fn decrypt_block(&self, block: &mut [u8; AES_BLOCK_SIZE]) {
        add_round_key(block, self.round_key(14));

        for round in (1..14).rev() {
            inv_shift_rows(block);
            inv_sub_bytes(block);
            add_round_key(block, self.round_key(round));
            inv_mix_columns(block);
        }

        inv_shift_rows(block);
        inv_sub_bytes(block);
        add_round_key(block, self.round_key(0));
    }

    fn round_key(&self, round: usize) -> &[u8; AES_BLOCK_SIZE] {
        let off = round * AES_BLOCK_SIZE;
        unsafe { &*(self.round_keys.as_ptr().add(off) as *const [u8; AES_BLOCK_SIZE]) }
    }
}

// ── XTS 模式 ──

/// AES-256-XTS 加解密器
pub struct AesXts {
    key1: Aes256Key,   // 加密/解密密钥
    key2: Aes256Key,   // tweak 密钥
}

impl AesXts {
    /// 从 64 字节 (512 位) XTS 密钥创建
    pub fn from_xts_key(key: &[u8; XTS_KEY_SIZE]) -> Self {
        let mut k1 = [0u8; AES_256_KEY_SIZE];
        let mut k2 = [0u8; AES_256_KEY_SIZE];
        k1.copy_from_slice(&key[0..32]);
        k2.copy_from_slice(&key[32..64]);
        AesXts {
            key1: Aes256Key::from_key(&k1),
            key2: Aes256Key::from_key(&k2),
        }
    }

    /// XTS 加密数据 (in-place)
    /// tweak: 128 位 tweak 值 (页索引)
    pub fn encrypt(&self, data: &mut [u8], tweak: u128) {
        if data.len() < AES_BLOCK_SIZE {
            return;
        }

        let mut t = tweak.to_le_bytes();
        self.key2.encrypt_block(&mut t);

        let full_blocks = data.len() / AES_BLOCK_SIZE;
        let remainder = data.len() % AES_BLOCK_SIZE;

        for i in 0..full_blocks {
            let block_off = i * AES_BLOCK_SIZE;
            // T = E(key2, tweak) × α^i
            let mut block_tweak = t;
            for _ in 0..i {
                gf_mul128(&mut block_tweak);
            }

            // XOR tweak → encrypt → XOR tweak
            xor_block(&mut data[block_off..block_off + AES_BLOCK_SIZE], &block_tweak);
            let block = unsafe {
                &mut *(data.as_mut_ptr().add(block_off) as *mut [u8; AES_BLOCK_SIZE])
            };
            self.key1.encrypt_block(block);
            xor_block(&mut data[block_off..block_off + AES_BLOCK_SIZE], &block_tweak);
        }

        // 密文窃取 (ciphertext stealing) for partial block
        if remainder > 0 {
            let last_full = (full_blocks.saturating_sub(1)) * AES_BLOCK_SIZE;
            let mut last_tweak = t;
            for _ in 0..full_blocks.saturating_sub(1) {
                gf_mul128(&mut last_tweak);
            }

            if full_blocks > 0 {
                // 交换最后完整块和部分块: 密文窃取
                let mut stolen = [0u8; AES_BLOCK_SIZE];
                stolen[..remainder].copy_from_slice(&data[last_full..last_full + remainder]);
                stolen[remainder..].fill(0);

                xor_last_block(&mut stolen, &last_tweak, remainder);
                {
                    let block = unsafe { &mut *(stolen.as_mut_ptr() as *mut [u8; AES_BLOCK_SIZE]) };
                    self.key1.encrypt_block(block);
                }
                xor_last_block(&mut stolen, &last_tweak, remainder);

                // 将密文窃取结果放回
                data[last_full..last_full + remainder].copy_from_slice(&stolen[..remainder]);
            } else {
                // 单个部分块: 直接 XTS (无窃取)
                let mut partial = [0u8; AES_BLOCK_SIZE];
                partial[..remainder].copy_from_slice(&data[..remainder]);
                xor_last_block(&mut partial, &t, remainder);
                self.key1.encrypt_block(&mut partial);
                xor_last_block(&mut partial, &t, remainder);
                data[..remainder].copy_from_slice(&partial[..remainder]);
            }
        }
    }

    /// XTS 解密数据 (in-place)
    /// tweak: 128 位 tweak 值 (页索引, 须与加密时相同)
    pub fn decrypt(&self, data: &mut [u8], tweak: u128) {
        if data.len() < AES_BLOCK_SIZE {
            return;
        }

        let mut t = tweak.to_le_bytes();
        self.key2.encrypt_block(&mut t);

        let full_blocks = data.len() / AES_BLOCK_SIZE;
        let remainder = data.len() % AES_BLOCK_SIZE;

        for i in 0..full_blocks {
            let block_off = i * AES_BLOCK_SIZE;
            let mut block_tweak = t;
            for _ in 0..i {
                gf_mul128(&mut block_tweak);
            }

            xor_block(&mut data[block_off..block_off + AES_BLOCK_SIZE], &block_tweak);
            let block = unsafe {
                &mut *(data.as_mut_ptr().add(block_off) as *mut [u8; AES_BLOCK_SIZE])
            };
            self.key1.decrypt_block(block);
            xor_block(&mut data[block_off..block_off + AES_BLOCK_SIZE], &block_tweak);
        }

        if remainder > 0 {
            let last_full = (full_blocks.saturating_sub(1)) * AES_BLOCK_SIZE;
            let mut last_tweak = t;
            for _ in 0..full_blocks.saturating_sub(1) {
                gf_mul128(&mut last_tweak);
            }

            if full_blocks > 0 {
                let mut stolen = [0u8; AES_BLOCK_SIZE];
                stolen[..remainder].copy_from_slice(&data[last_full..last_full + remainder]);
                xor_last_block(&mut stolen, &last_tweak, remainder);
                {
                    let block = unsafe { &mut *(stolen.as_mut_ptr() as *mut [u8; AES_BLOCK_SIZE]) };
                    self.key1.decrypt_block(block);
                }
                xor_last_block(&mut stolen, &last_tweak, remainder);
                data[last_full..last_full + remainder].copy_from_slice(&stolen[..remainder]);
            } else {
                let mut partial = [0u8; AES_BLOCK_SIZE];
                partial[..remainder].copy_from_slice(&data[..remainder]);
                xor_last_block(&mut partial, &t, remainder);
                self.key1.decrypt_block(&mut partial);
                xor_last_block(&mut partial, &t, remainder);
                data[..remainder].copy_from_slice(&partial[..remainder]);
            }
        }
    }
}

// ── GF(2^128) 辅助 (用于 XTS tweak 乘法) ──

/// GF(2^128) 乘以 α (即乘以 x)
fn gf_mul128(block: &mut [u8; 16]) {
    let mut carry = 0u8;
    for i in 0..16 {
        let byte = block[i];
        block[i] = (byte << 1) | carry;
        carry = (byte >> 7) & 1;
    }
    if carry != 0 {
        block[0] ^= 0x87; // 不可约多项式的低字节
    }
}

fn xor_block(data: &mut [u8], tweak: &[u8; 16]) {
    for i in 0..data.len().min(16) {
        data[i] ^= tweak[i];
    }
}

fn xor_last_block(data: &mut [u8; 16], tweak: &[u8; 16], len: usize) {
    for i in 0..len {
        data[i] ^= tweak[i];
    }
}

// ── 文件加密 API ──

/// 为 inode 初始化加密上下文
pub fn init_encryption(ino: Ino, master_key_id: &[u8; 8]) -> FsResult<EncryptContext> {
    let mut ctx = EncryptContext::empty();
    ctx.version = 1;
    ctx.algorithm = ENCRYPT_ALG_AES256_XTS;
    ctx.key_id.copy_from_slice(master_key_id);
    // 生成随机 salt (简化: 用 ino + 固定值)
    ctx.salt[0..8].copy_from_slice(&ino.to_le_bytes());
    ctx.salt[8..16].copy_from_slice(b"Tungsten");
    ctx.flags = 0;

    // 存储加密上下文到 inode
    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
    di.flags |= FS_ENCRYPTED_FL;
    unsafe {
        core::ptr::copy_nonoverlapping(
            &ctx as *const EncryptContext as *const u8,
            di.encrypt_ctx.as_mut_ptr(), 32,
        );
    }
    write_disk_inode(ino, &di).map_err(|_| FsError::Eio)?;

    Ok(ctx)
}

/// 派生每文件加密密钥 (HKDF 简化: HMAC-SHA256 的简化替代)
/// master_key XOR (salt || key_id) → XTS key
pub fn derive_file_key(master_key: &[u8; 32], ctx: &EncryptContext) -> [u8; XTS_KEY_SIZE] {
    let mut xts_key = [0u8; XTS_KEY_SIZE];

    // 简化密钥派生: key1 = HKDF-extract(master, salt), key2 = HKDF-expand(key1, key_id)
    // Phase 5 用 XOR+哈希模拟, 后续替换为完整 HKDF-SHA256
    for i in 0..32 {
        xts_key[i] = master_key[i] ^ ctx.salt[i % 16];
    }
    for i in 0..32 {
        xts_key[32 + i] = master_key[i] ^ ctx.key_id[i % 8] ^ 0x5A;
    }

    xts_key
}

/// 获取文件的 XTS 加解密器 (如果文件已加密)
pub fn get_file_cipher(ino: Ino, master_key: &[u8; 32]) -> FsResult<Option<AesXts>> {
    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;

    if di.flags & FS_ENCRYPTED_FL == 0 {
        return Ok(None);
    }

    let ctx: EncryptContext = unsafe {
        core::ptr::read_unaligned(di.encrypt_ctx.as_ptr() as *const EncryptContext)
    };

    if !ctx.is_valid() {
        return Ok(None);
    }

    let xts_key = derive_file_key(master_key, &ctx);
    Ok(Some(AesXts::from_xts_key(&xts_key)))
}

/// 加密一个页面的数据
/// page_index: 文件内页索引 (用作 XTS tweak)
pub fn encrypt_page_data(ino: Ino, page_index: u64, data: &mut [u8], master_key: &[u8; 32]) -> FsResult<()> {
    if let Some(cipher) = get_file_cipher(ino, master_key)? {
        cipher.encrypt(data, page_index as u128);
    }
    Ok(())
}

/// 解密一个页面的数据
pub fn decrypt_page_data(ino: Ino, page_index: u64, data: &mut [u8], master_key: &[u8; 32]) -> FsResult<()> {
    if let Some(cipher) = get_file_cipher(ino, master_key)? {
        cipher.decrypt(data, page_index as u128);
    }
    Ok(())
}

// ── 全局主密钥 ──

use core::cell::UnsafeCell;

struct MasterKeyWrapper(UnsafeCell<[u8; 32]>);
unsafe impl Sync for MasterKeyWrapper {}

static MASTER_KEY: MasterKeyWrapper = MasterKeyWrapper(UnsafeCell::new([0u8; 32]));

/// 设置系统主加密密钥
pub fn set_master_key(key: &[u8; 32]) {
    unsafe { *MASTER_KEY.0.get() = *key; }
}

/// 获取系统主加密密钥
pub fn get_master_key() -> [u8; 32] {
    unsafe { *MASTER_KEY.0.get() }
}
