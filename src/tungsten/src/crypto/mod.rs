// crypto/mod.rs — 加密子系统 (AES/SHA/RSA/ECDSA/ChaCha20)
// 内核加密 API、随机数生成、密钥管理、TPM 集成
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod aes; pub mod sha256; pub mod sha512; pub mod rsa; pub mod ecdsa;
pub mod chacha20; pub mod poly1305; pub mod hmac; pub mod kdf; pub mod drbg;

use crate::sync::SpinLock;
pub struct CryptoManager { pub rng_seeded: bool, pub fips_mode: bool }
static CRYPTO_MGR: SpinLock<CryptoManager> = SpinLock::new(CryptoManager { rng_seeded: false, fips_mode: false });
pub fn init() { aes::init(); sha256::init(); chacha20::init(); drbg::seed();
    crate::serial::write_str(b"crypto: subsystem ready\n"); }
