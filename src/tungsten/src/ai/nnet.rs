// ai/nnet.rs — 神经网络基础运算
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::f32;

/// vocab 大小（ASCII 可打印字符范围）
pub const VOCAB: usize = 95;

/// 字符 → 索引（可打印 ASCII: 32..=126）
pub const fn char_to_idx(c: u8) -> usize {
    if c >= 32 && c <= 126 { (c - 32) as usize } else { 0 }
}

/// 索引 → 字符
pub fn idx_to_char(i: usize) -> u8 {
    if i < VOCAB { (i as u8) + 32 } else { b' ' }
}

/// 手动 floor（no_std 下 f32 无此方法）
fn floor_f32(x: f32) -> f32 {
    let xi = x as i32;
    if x >= 0.0 || x == xi as f32 {
        xi as f32
    } else {
        (xi.saturating_sub(1)) as f32
    }
}

/// 手动 powi（no_std 下 f32 无此方法）
fn powi_f32(base: f32, exp: i32) -> f32 {
    if exp == 0 {
        return 1.0;
    }
    let mut result = 1.0f32;
    let mut b = base;
    let mut e = exp.abs();
    loop {
        if e & 1 == 1 {
            result *= b;
        }
        e >>= 1;
        if e == 0 {
            break;
        }
        b *= b;
    }
    if exp < 0 { 1.0 / result } else { result }
}

/// 手动 exp（no_std 下 f32 无此方法，使用泰勒展开 + powi 分解）
fn exp(x: f32) -> f32 {
    const LN2: f32 = core::f32::consts::LN_2;
    let scaled = x / LN2;
    let n = floor_f32(scaled);
    let r = x - n * LN2;
    // exp(r) 的 5 阶泰勒展开，|r| < ln2/2 时精度足够
    let exp_r = 1.0 + r * (1.0 + r * (0.5 + r * (1.0/6.0 + r * (1.0/24.0 + r * (1.0/120.0)))));
    exp_r * powi_f32(2.0, n as i32)
}

/// softmax: 将 logits 转换为概率分布（原地修改）
pub fn softmax(logits: &mut [f32]) {
    let max = logits.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let sum: f32 = logits.iter_mut().map(|x| {
        *x = exp(*x - max);
        *x
    }).sum();
    for x in logits.iter_mut() {
        *x /= sum;
    }
}

/// 从概率分布采样
/// `r` 为 [0,1) 内的采样阈值（固定 0.7，后续可接入真实 RNG）
pub fn sample(probs: &[f32]) -> usize {
    let r = 0.7;
    let mut cum: f32 = 0.0;
    for (i, &p) in probs.iter().enumerate() {
        cum += p;
        if r <= cum { return i; }
    }
    probs.len() - 1
}

/// 线性层：y = x * W^T + b（x: 行向量, W: [in×out], b: [out]）
pub fn linear(x: &[f32], w: &[f32], b: &[f32], out: &mut [f32]) {
    let in_dim = x.len();
    let out_dim = out.len();
    for o in 0..out_dim {
        let mut sum = b[o];
        for i in 0..in_dim {
            sum += x[i] * w[o * in_dim + i];
        }
        out[o] = sum;
    }
}

/// ReLU 激活（原地）
pub fn relu(x: &mut [f32]) {
    for v in x.iter_mut() {
        if *v < 0.0 { *v = 0.0; }
    }
}
