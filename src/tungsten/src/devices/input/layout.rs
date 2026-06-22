// devices/input/layout.rs — 键盘布局系统
//
// 支持多布局运行时切换，每个布局定义符号键映射。
// 字母键 A-Z 采用统一映射 (所有布局相同)，
// 仅符号键和数字行在不同布局中有差异。
//
// 布局: US (en-US), UK (en-GB), DE (de-DE), FR (fr-FR)
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::devices::input::keycode;
use keycode::KeyCode;

/* ── 布局标识 ── */

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KbdLayout {
    Us = 0,
    Uk = 1,
    De = 2,
    Fr = 3,
}

/// 布局名称
pub const LAYOUT_NAMES: [&str; 4] = ["us", "uk", "de", "fr"];

/// 当前活动布局 (全局)
static mut CURRENT_LAYOUT: KbdLayout = KbdLayout::Us;

/// 设置活动布局
pub fn set_layout(layout: KbdLayout) {
    unsafe { CURRENT_LAYOUT = layout; }
    crate::serial::write_str(b"layout: switched to ");
    crate::serial::write_str(LAYOUT_NAMES[layout as usize].as_bytes());
    crate::serial::write_str(b"\n");
}

/// 获取当前布局
pub fn current_layout() -> KbdLayout {
    unsafe { CURRENT_LAYOUT }
}

/// 按名称查找布局
pub fn layout_by_name(name: &str) -> Option<KbdLayout> {
    match name {
        "us" | "en" | "en-US" | "en-us" => Some(KbdLayout::Us),
        "uk" | "en-GB" | "en-gb" | "gb" => Some(KbdLayout::Uk),
        "de" | "de-DE" | "de-de" | "german" => Some(KbdLayout::De),
        "fr" | "fr-FR" | "fr-fr" | "french" => Some(KbdLayout::Fr),
        _ => None,
    }
}

/* ── 布局映射表 ── */

/// 符号键映射条目: (keycode, base_char, shifted_char)
type SymMap = &'static [(KeyCode, u8, u8)];

/// US 布局符号键映射
static US_SYMS: SymMap = &[
    (keycode::KEY_MINUS,      b'-', b'_'),
    (keycode::KEY_EQUAL,      b'=', b'+'),
    (keycode::KEY_LEFTBRACE,  b'[', b'{'),
    (keycode::KEY_RIGHTBRACE, b']', b'}'),
    (keycode::KEY_BACKSLASH,  b'\\', b'|'),
    (keycode::KEY_SEMICOLON,  b';', b':'),
    (keycode::KEY_APOSTROPHE, b'\'', b'"'),
    (keycode::KEY_GRAVE,      b'`', b'~'),
    (keycode::KEY_COMMA,      b',', b'<'),
    (keycode::KEY_DOT,        b'.', b'>'),
    (keycode::KEY_SLASH,      b'/', b'?'),
    (keycode::KEY_HASHTILDE,  b'#', b'~'),
];

/// UK 布局符号键映射
static UK_SYMS: SymMap = &[
    (keycode::KEY_MINUS,      b'-', b'_'),
    (keycode::KEY_EQUAL,      b'=', b'+'),
    (keycode::KEY_LEFTBRACE,  b'[', b'{'),
    (keycode::KEY_RIGHTBRACE, b']', b'}'),
    (keycode::KEY_BACKSLASH,  b'#', b'~'),
    (keycode::KEY_SEMICOLON,  b';', b':'),
    (keycode::KEY_APOSTROPHE, b'\'', b'@'),
    (keycode::KEY_GRAVE,      b'`', b'\xA4'),
    (keycode::KEY_COMMA,      b',', b'<'),
    (keycode::KEY_DOT,        b'.', b'>'),
    (keycode::KEY_SLASH,      b'/', b'?'),
    (keycode::KEY_HASHTILDE,  b'\\', b'|'),
];

/// DE (German QWERTZ) 符号键映射
static DE_SYMS: SymMap = &[
    (keycode::KEY_MINUS,      b'\xDF', b'?'),
    (keycode::KEY_EQUAL,      b'\'', b'`'),
    (keycode::KEY_LEFTBRACE,  b'\xFC', b'\xDC'),
    (keycode::KEY_RIGHTBRACE, b'+', b'*'),
    (keycode::KEY_BACKSLASH,  b'#', b'\''),
    (keycode::KEY_SEMICOLON,  b'\xF6', b'\xD6'),
    (keycode::KEY_APOSTROPHE, b'\xE4', b'\xC4'),
    (keycode::KEY_GRAVE,      b'^', b'\xB0'),
    (keycode::KEY_COMMA,      b',', b';'),
    (keycode::KEY_DOT,        b'.', b':'),
    (keycode::KEY_SLASH,      b'-', b'_'),
    (keycode::KEY_HASHTILDE,  b'<', b'>'),
];

/// FR (French AZERTY) 符号键映射
static FR_SYMS: SymMap = &[
    (keycode::KEY_MINUS,      b')', b'\xB0'),
    (keycode::KEY_EQUAL,      b'=', b'+'),
    (keycode::KEY_LEFTBRACE,  b'^', b'\xA8'),
    (keycode::KEY_RIGHTBRACE, b'$', b'\xA3'),
    (keycode::KEY_BACKSLASH,  b'*', b'\xFC'),
    (keycode::KEY_SEMICOLON,  b'm', b'M'),
    (keycode::KEY_APOSTROPHE, b'\xF9', b'\xD9'),
    (keycode::KEY_GRAVE,      b'\xE9', b'\xC9'),
    (keycode::KEY_COMMA,      b'?', b'.'),
    (keycode::KEY_DOT,        b';', b','),
    (keycode::KEY_SLASH,      b'!', b'\xA7'),
    (keycode::KEY_HASHTILDE,  b'<', b'>'),
];

/// 数字行映射 (部分布局数字行输出不同)
type NumMap = &'static [(u8, u8)];

/// US 数字行映射
static NUM_US: NumMap = &[
    (b'1', b'!'), (b'2', b'@'), (b'3', b'#'), (b'4', b'$'),
    (b'5', b'%'), (b'6', b'^'), (b'7', b'&'), (b'8', b'*'),
    (b'9', b'('), (b'0', b')'),
];

/// FR AZERTY 数字行映射 (数字需要 Shift)
static NUM_FR: NumMap = &[
    (b'1', b'&'), (b'2', b'\xE9'), (b'3', b'"'), (b'4', b'\''),
    (b'5', b'('), (b'6', b'-'),    (b'7', b'\xE8'), (b'8', b'_'),
    (b'9', b'\xE7'), (b'0', b'\xE0'),
];

/* ── 字符转换 (布局感知) ── */

/// 将键码转换为 ASCII/Unicode 字符，考虑布局和修饰键状态。
/// 返回 Some(u8) 如果产生可打印字符 (拉丁-1 范围)。
pub fn keycode_to_char(key: KeyCode, caps: bool, shift: bool, layout: KbdLayout) -> Option<u8> {
    // 字母 A-Z: 所有布局统一，受 Caps 和 Shift 影响
    if (0x04..=0x1D).contains(&key) {
        let c = (b'a' + (key - 0x04) as u8) as char;
        let upper = shift ^ caps;
        return Some(if upper { c.to_ascii_uppercase() as u8 } else { c as u8 });
    }

    // 数字行 (0x1E-0x27)
    if (0x1E..=0x27).contains(&key) {
        let num_idx = (key - 0x1E) as usize;
        let (base, shifted) = match layout {
            KbdLayout::Fr => NUM_FR[num_idx],
            _ => NUM_US[num_idx],
        };
        return Some(if shift { shifted } else { base });
    }

    // 符号键: 按布局查表
    let syms = match layout {
        KbdLayout::Us => US_SYMS,
        KbdLayout::Uk => UK_SYMS,
        KbdLayout::De => DE_SYMS,
        KbdLayout::Fr => FR_SYMS,
    };

    for &(k, base, shifted) in syms.iter() {
        if k == key {
            return Some(if shift { shifted } else { base });
        }
    }

    // 空格 / 回车 / Tab / 退格 / Esc
    match key {
        keycode::KEY_SPACE     => Some(b' '),
        keycode::KEY_TAB       => Some(b'\t'),
        keycode::KEY_ENTER     => Some(b'\n'),
        keycode::KEY_BACKSPACE => Some(b'\x7f'),
        keycode::KEY_ESCAPE    => Some(b'\x1b'),
        _ => None,
    }
}

/// 输出当前布局名称到串口 (调试用)
pub fn print_layout() {
    let name = LAYOUT_NAMES[current_layout() as usize];
    crate::serial::write_str(b"layout: ");
    crate::serial::write_str(name.as_bytes());
    crate::serial::write_str(b"\n");
}
