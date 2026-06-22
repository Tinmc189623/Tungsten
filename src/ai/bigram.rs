// ai/bigram.rs — 字符级 bigram 语言模型
// 基于 95×95 概率矩阵的下一字符预测
// 权重在编译时初始化（从训练文本统计）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::nnet::{VOCAB, char_to_idx, idx_to_char};

/// bigram 概率矩阵 [VOCAB][VOCAB] — 行归一化的条件概率 P(c2|c1)
struct BigramModel {
    logits: [[f32; VOCAB]; VOCAB],
}

static MODEL: BigramModel = BigramModel::new();

impl BigramModel {
    /// 从训练文本统计构建 bigram 概率表
    const fn new() -> Self {
        let mut logits = [[0.0f32; VOCAB]; VOCAB];
        let text = b"TungstenOS is an AI-native operating system. The kernel is called Tungsten. It uses a four-ring privilege architecture. Everything is an object. Welcome to the future of operating systems. ";
        let mut i = 0;
        while i < text.len() - 1 {
            let c1 = char_to_idx(text[i]);
            let c2 = char_to_idx(text[i + 1]);
            logits[c1][c2] += 1.0;
            i += 1;
        }
        // Laplace 平滑 + 归一化
        let mut row = 0;
        while row < VOCAB {
            let mut sum = 0.0f32;
            let mut col = 0;
            while col < VOCAB {
                logits[row][col] += 1.0;
                sum += logits[row][col];
                col += 1;
            }
            col = 0;
            while col < VOCAB {
                logits[row][col] /= sum;
                col += 1;
            }
            row += 1;
        }
        BigramModel { logits }
    }
}

/// 生成文本：给定前缀，生成最多 max_len 个字符
pub fn generate(input: &[u8], output: &mut [u8]) -> usize {
    let max_len = output.len().min(256);
    if input.is_empty() {
        let start = char_to_idx(b'T');
        let probs = MODEL.logits[start];
        let next = softmax_sample(&probs);
        output[0] = idx_to_char(next);
        return 1;
    }

    let mut last = char_to_idx(input[input.len().saturating_sub(1)]);
    let mut written = 0;

    // 复制输入到输出
    for &c in input.iter().take(max_len) {
        output[written] = c;
        written += 1;
    }

    // 自回归生成
    while written < max_len {
        let probs = MODEL.logits[last];
        let next = softmax_sample(&probs);
        let c = idx_to_char(next);
        output[written] = c;
        written += 1;
        last = next;
        if c == b'.' || c == b'\n' { break; }
    }
    written
}

fn softmax_sample(probs: &[f32; VOCAB]) -> usize {
    let r = 0.85;
    let mut cum: f32 = 0.0;
    for (i, &p) in probs.iter().enumerate() {
        cum += p;
        if r <= cum { return i; }
    }
    VOCAB - 1
}
