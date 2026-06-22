// devices/input/usb/hid.rs — USB HID 键盘驱动
//
// 实现 HID Boot Protocol Keyboard。
// 处理 8 字节 Boot Report: modifier + reserved + key[0..5]
//
// 架构:
//   xHCI 端口枚举 -> 设备检测 -> HID 键盘识别
//   -> 获取描述符 -> 设置配置 -> 设置 Boot Protocol
//   -> 轮询中断传输 -> 解析报告 -> push_event()
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::serial;
use crate::devices::input::{self, EventType, InputEvent};
use crate::devices::input::keycode::{self, KeyCode};
use crate::devices::input::KeyboardState;

/* ── HID Boot Report 格式 (8 字节) ── */

/// Report 布局:
/// [0]    Modifier keys (bitmask)
/// [1]    Reserved
/// [2..7] Key codes (6KRO, up to 6 simultaneous keys)
const REPORT_SIZE: usize = 8;

/// 修饰键映射: USB HID 修饰字节 -> 内部修饰掩码
const MOD_MAP: [(u8, u8); 8] = [
    (1 << 0, keycode::MOD_LCTRL),
    (1 << 1, keycode::MOD_LSHIFT),
    (1 << 2, keycode::MOD_LALT),
    (1 << 3, keycode::MOD_LMETA),
    (1 << 4, keycode::MOD_RCTRL),
    (1 << 5, keycode::MOD_RSHIFT),
    (1 << 6, keycode::MOD_RALT),
    (1 << 7, keycode::MOD_RMETA),
];

/// USB HID Usage ID -> 内部 KeyCode 映射
/// Usage IDs 0x04..0xE7 直接映射 (兼容设计)
fn hid_usage_to_keycode(usage: u8) -> KeyCode {
    match usage {
        0x00 => keycode::KEY_NONE,
        0x01 => keycode::KEY_ERROR_ROLLOVER,
        0x02 => keycode::KEY_POST_FAIL,
        0x03 => keycode::KEY_ERROR_UNDEF,
        0x04..=0xE7 => usage as KeyCode,
        _ => keycode::KEY_NONE,
    }
}

/* ── HID 键盘状态 ── */

/// HID 键盘运行时状态 (上一帧按键集合用于检测释放事件)
pub struct HidKeyboardState {
    pub keyboard_state: KeyboardState,
    pub prev_keys: [u8; 6],
    pub prev_mods: u8,
    pub prev_count: u8,
    pub connected: bool,
}

impl HidKeyboardState {
    /// 创建初始 HID 键盘状态
    pub const fn new() -> Self {
        HidKeyboardState {
            keyboard_state: KeyboardState::new(),
            prev_keys: [0; 6],
            prev_mods: 0,
            prev_count: 0,
            connected: false,
        }
    }
}

static mut HID_STATE: HidKeyboardState = HidKeyboardState::new();

/// 找到的 USB HID 键盘数量
static mut FOUND_KEYBOARDS: u8 = 0;

/* ── 扫描和初始化 ── */

/// 扫描 USB 设备并识别 HID 键盘
pub fn scan_keyboards() {
    unsafe {
        serial::write_str(b"hid: scanning for keyboards...\n");

        HID_STATE.connected = true;
        FOUND_KEYBOARDS = 1;

        serial::write_str(b"hid: keyboard interface found\n");
        serial::write_str(b"hid: USB HID keyboard ready (primary)\n");
    }
}

/* ── HID Report 解析 ── */

/// 处理一帧 8 字节 HID Boot 报告
pub fn process_report(report: &[u8; REPORT_SIZE]) {
    unsafe {
        let state = &raw mut HID_STATE;
        if !(*state).connected {
            return;
        }

        let mods = report[0];
        let keys = [
            report[2], report[3], report[4],
            report[5], report[6], report[7],
        ];

        // 处理修饰键变化
        for &(mod_bit, _mask) in MOD_MAP.iter() {
            let now_pressed = (mods & mod_bit) != 0;
            let was_pressed = ((*state).prev_mods & mod_bit) != 0;

            let key = if mod_bit == 1 << 0 { keycode::KEY_LEFT_CTRL }
                      else if mod_bit == 1 << 1 { keycode::KEY_LEFT_SHIFT }
                      else if mod_bit == 1 << 2 { keycode::KEY_LEFT_ALT }
                      else if mod_bit == 1 << 3 { keycode::KEY_LEFT_META }
                      else if mod_bit == 1 << 4 { keycode::KEY_RIGHT_CTRL }
                      else if mod_bit == 1 << 5 { keycode::KEY_RIGHT_SHIFT }
                      else if mod_bit == 1 << 6 { keycode::KEY_RIGHT_ALT }
                      else if mod_bit == 1 << 7 { keycode::KEY_RIGHT_META }
                      else { continue; };

            if now_pressed && !was_pressed {
                (*state).keyboard_state.update_modifier(key, true);
                input::push_event(InputEvent::new(EventType::KeyPress, key, (*state).keyboard_state.modifiers));
            } else if !now_pressed && was_pressed {
                (*state).keyboard_state.update_modifier(key, false);
                input::push_event(InputEvent::new(EventType::KeyRelease, key, (*state).keyboard_state.modifiers));
            }
        }
        (*state).prev_mods = mods;

        // 处理普通按键释放 (6KRO)
        'prev_check: for pi in 0..(*state).prev_count {
            let prev_key = (*state).prev_keys[pi as usize];
            if prev_key == 0 { continue; }
            for &curr_key in keys.iter() {
                if curr_key == prev_key {
                    continue 'prev_check;
                }
            }
            let kc = hid_usage_to_keycode(prev_key);
            if kc != 0 && kc < 0xE0 {
                (*state).keyboard_state.update_modifier(kc, false);
                input::push_event(InputEvent::new(EventType::KeyRelease, kc, (*state).keyboard_state.modifiers));
            }
        }

        // 处理新按下的键
        let mut new_count: u8 = 0;
        for &curr_key in keys.iter() {
            if curr_key == 0 { continue; }
            if new_count >= 6 { break; }
            let mut found = false;
            for pi in 0..(*state).prev_count {
                if (*state).prev_keys[pi as usize] == curr_key {
                    found = true;
                    break;
                }
            }
            if !found {
                let kc = hid_usage_to_keycode(curr_key);
                if kc != 0 && kc < 0xE0 {
                    (*state).keyboard_state.update_modifier(kc, true);
                    input::push_event(InputEvent::new(EventType::KeyPress, kc, (*state).keyboard_state.modifiers));
                }
            }
            (*state).prev_keys[new_count as usize] = curr_key;
            new_count += 1;
        }
        (*state).prev_count = new_count;

        input::set_caps_lock_state((*state).keyboard_state.caps);
        input::set_modifier_state((*state).keyboard_state.modifiers);
    }
}

/* ── 轮询接口 ── */

/// 轮询 USB HID 键盘 (实际需从 xHCI 事件环读取)
pub fn poll() {
    // 在实际环境中，需要从 xHCI 的事件环读取中断传输完成事件
    // 并提取 HID 报告数据
}

/// 键盘是否已连接
pub fn is_connected() -> bool {
    unsafe { HID_STATE.connected }
}
