// ai/mod.rs — 内核级 AI 推理引擎
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod nnet;
pub mod bigram;

/// 推理入口：输入文本，输出生成文本
pub fn infer(input: &[u8], output: &mut [u8]) -> usize {
    bigram::generate(input, output)
}
