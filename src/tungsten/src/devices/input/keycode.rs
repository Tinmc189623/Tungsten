// devices/input/keycode.rs — 统一键码定义 (基于 USB HID 标准 Usage ID)
//
// 所有输入驱动 (PS/2、USB HID) 都转换为此套键码。
// 上层消费方 (shell、GUI) 仅依赖此键码，不关心底层扫描码。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



/// 键码类型 (USB HID Usage ID 兼容)
pub type KeyCode = u16;

/* ── 修饰键掩码 ── */

pub const MOD_LCTRL:  u8 = 0x01;
pub const MOD_LSHIFT: u8 = 0x02;
pub const MOD_LALT:   u8 = 0x04;
pub const MOD_LMETA:  u8 = 0x08;
pub const MOD_RCTRL:  u8 = 0x10;
pub const MOD_RSHIFT: u8 = 0x20;
pub const MOD_RALT:   u8 = 0x40;
pub const MOD_RMETA:  u8 = 0x80;

/* ── 键码值 (USB HID Usage ID 兼容) ── */

pub const KEY_NONE:        KeyCode = 0x00;
pub const KEY_ERROR_ROLLOVER: KeyCode = 0x01;
pub const KEY_POST_FAIL:   KeyCode = 0x02;
pub const KEY_ERROR_UNDEF: KeyCode = 0x03;

/* 字母 A-Z */
pub const KEY_A: KeyCode = 0x04;
pub const KEY_B: KeyCode = 0x05;
pub const KEY_C: KeyCode = 0x06;
pub const KEY_D: KeyCode = 0x07;
pub const KEY_E: KeyCode = 0x08;
pub const KEY_F: KeyCode = 0x09;
pub const KEY_G: KeyCode = 0x0A;
pub const KEY_H: KeyCode = 0x0B;
pub const KEY_I: KeyCode = 0x0C;
pub const KEY_J: KeyCode = 0x0D;
pub const KEY_K: KeyCode = 0x0E;
pub const KEY_L: KeyCode = 0x0F;
pub const KEY_M: KeyCode = 0x10;
pub const KEY_N: KeyCode = 0x11;
pub const KEY_O: KeyCode = 0x12;
pub const KEY_P: KeyCode = 0x13;
pub const KEY_Q: KeyCode = 0x14;
pub const KEY_R: KeyCode = 0x15;
pub const KEY_S: KeyCode = 0x16;
pub const KEY_T: KeyCode = 0x17;
pub const KEY_U: KeyCode = 0x18;
pub const KEY_V: KeyCode = 0x19;
pub const KEY_W: KeyCode = 0x1A;
pub const KEY_X: KeyCode = 0x1B;
pub const KEY_Y: KeyCode = 0x1C;
pub const KEY_Z: KeyCode = 0x1D;

/* 数字 1-9, 0 */
pub const KEY_1: KeyCode = 0x1E;
pub const KEY_2: KeyCode = 0x1F;
pub const KEY_3: KeyCode = 0x20;
pub const KEY_4: KeyCode = 0x21;
pub const KEY_5: KeyCode = 0x22;
pub const KEY_6: KeyCode = 0x23;
pub const KEY_7: KeyCode = 0x24;
pub const KEY_8: KeyCode = 0x25;
pub const KEY_9: KeyCode = 0x26;
pub const KEY_0: KeyCode = 0x27;

/* 回车 / 退格 / Tab / 空格 */
pub const KEY_ENTER:      KeyCode = 0x28;
pub const KEY_ESCAPE:     KeyCode = 0x29;
pub const KEY_BACKSPACE:  KeyCode = 0x2A;
pub const KEY_TAB:        KeyCode = 0x2B;
pub const KEY_SPACE:      KeyCode = 0x2C;

/* 符号键 */
pub const KEY_MINUS:      KeyCode = 0x2D;
pub const KEY_EQUAL:      KeyCode = 0x2E;
pub const KEY_LEFTBRACE:  KeyCode = 0x2F;
pub const KEY_RIGHTBRACE: KeyCode = 0x30;
pub const KEY_BACKSLASH:  KeyCode = 0x31;
pub const KEY_HASHTILDE:  KeyCode = 0x32;
pub const KEY_SEMICOLON:  KeyCode = 0x33;
pub const KEY_APOSTROPHE: KeyCode = 0x34;
pub const KEY_GRAVE:      KeyCode = 0x35;
pub const KEY_COMMA:      KeyCode = 0x36;
pub const KEY_DOT:        KeyCode = 0x37;
pub const KEY_SLASH:      KeyCode = 0x38;

/* 大写锁定 */
pub const KEY_CAPSLOCK:   KeyCode = 0x39;

/* F1-F12 */
pub const KEY_F1:  KeyCode = 0x3A;
pub const KEY_F2:  KeyCode = 0x3B;
pub const KEY_F3:  KeyCode = 0x3C;
pub const KEY_F4:  KeyCode = 0x3D;
pub const KEY_F5:  KeyCode = 0x3E;
pub const KEY_F6:  KeyCode = 0x3F;
pub const KEY_F7:  KeyCode = 0x40;
pub const KEY_F8:  KeyCode = 0x41;
pub const KEY_F9:  KeyCode = 0x42;
pub const KEY_F10: KeyCode = 0x43;
pub const KEY_F11: KeyCode = 0x44;
pub const KEY_F12: KeyCode = 0x45;

/* 锁键 */
pub const KEY_SYSRQ:      KeyCode = 0x46;
pub const KEY_SCROLLLOCK: KeyCode = 0x47;
pub const KEY_PAUSE:      KeyCode = 0x48;

/* 导航键 */
pub const KEY_INSERT:     KeyCode = 0x49;
pub const KEY_HOME:       KeyCode = 0x4A;
pub const KEY_PAGEUP:     KeyCode = 0x4B;
pub const KEY_DELETE:     KeyCode = 0x4C;
pub const KEY_END:        KeyCode = 0x4D;
pub const KEY_PAGEDOWN:   KeyCode = 0x4E;
pub const KEY_RIGHT:      KeyCode = 0x4F;
pub const KEY_LEFT:       KeyCode = 0x50;
pub const KEY_DOWN:       KeyCode = 0x51;
pub const KEY_UP:         KeyCode = 0x52;

/* 数字键盘 */
pub const KEY_KP_NUMLOCK: KeyCode = 0x53;
pub const KEY_KP_SLASH:   KeyCode = 0x54;
pub const KEY_KP_ASTERISK:KeyCode = 0x55;
pub const KEY_KP_MINUS:   KeyCode = 0x56;
pub const KEY_KP_PLUS:    KeyCode = 0x57;
pub const KEY_KP_ENTER:   KeyCode = 0x58;
pub const KEY_KP_1: KeyCode = 0x59;
pub const KEY_KP_2: KeyCode = 0x5A;
pub const KEY_KP_3: KeyCode = 0x5B;
pub const KEY_KP_4: KeyCode = 0x5C;
pub const KEY_KP_5: KeyCode = 0x5D;
pub const KEY_KP_6: KeyCode = 0x5E;
pub const KEY_KP_7: KeyCode = 0x5F;
pub const KEY_KP_8: KeyCode = 0x60;
pub const KEY_KP_9: KeyCode = 0x61;
pub const KEY_KP_0: KeyCode = 0x62;
pub const KEY_KP_DOT:     KeyCode = 0x63;

/* 非 US 键盘 */
pub const KEY_102ND:      KeyCode = 0x64;
pub const KEY_COMPOSE:    KeyCode = 0x65;
pub const KEY_POWER:      KeyCode = 0x66;
pub const KEY_KP_EQUAL:   KeyCode = 0x67;

/* F13-F24 */
pub const KEY_F13: KeyCode = 0x68;
pub const KEY_F14: KeyCode = 0x69;
pub const KEY_F15: KeyCode = 0x6A;
pub const KEY_F16: KeyCode = 0x6B;
pub const KEY_F17: KeyCode = 0x6C;
pub const KEY_F18: KeyCode = 0x6D;
pub const KEY_F19: KeyCode = 0x6E;
pub const KEY_F20: KeyCode = 0x6F;
pub const KEY_F21: KeyCode = 0x70;
pub const KEY_F22: KeyCode = 0x71;
pub const KEY_F23: KeyCode = 0x72;
pub const KEY_F24: KeyCode = 0x73;

/* 附加功能键 */
pub const KEY_OPEN:          KeyCode = 0x74;
pub const KEY_HELP:          KeyCode = 0x75;
pub const KEY_MENU:          KeyCode = 0x76;
pub const KEY_SELECT:        KeyCode = 0x77;
pub const KEY_STOP:          KeyCode = 0x78;
pub const KEY_AGAIN:         KeyCode = 0x79;
pub const KEY_UNDO:          KeyCode = 0x7A;
pub const KEY_CUT:           KeyCode = 0x7B;
pub const KEY_COPY:          KeyCode = 0x7C;
pub const KEY_PASTE:         KeyCode = 0x7D;
pub const KEY_FIND:          KeyCode = 0x7E;
pub const KEY_MUTE:          KeyCode = 0x7F;
pub const KEY_VOLUMEUP:      KeyCode = 0x80;
pub const KEY_VOLUMEDOWN:    KeyCode = 0x81;

/* 左/右修饰键 (USB HID 报告中的单独键码) */
pub const KEY_LEFT_CTRL:    KeyCode = 0xE0;
pub const KEY_LEFT_SHIFT:   KeyCode = 0xE1;
pub const KEY_LEFT_ALT:     KeyCode = 0xE2;
pub const KEY_LEFT_META:    KeyCode = 0xE3;
pub const KEY_RIGHT_CTRL:   KeyCode = 0xE4;
pub const KEY_RIGHT_SHIFT:  KeyCode = 0xE5;
pub const KEY_RIGHT_ALT:    KeyCode = 0xE6;
pub const KEY_RIGHT_META:   KeyCode = 0xE7;

/* ── 键码到 ASCII 转换 (US 布局参考) ── */

/// 将键码 + 修饰键状态转换为 ASCII 字符。
/// 返回 Some(c) 如果该按键产生可打印字符，否则返回 None。
pub fn keycode_to_ascii(key: KeyCode, caps: bool, shift: bool) -> Option<u8> {
    // 字母: 受大写锁定和 Shift 影响
    if (0x04..=0x1D).contains(&key) {
        let c = (b'a' + (key - 0x04) as u8) as char;
        let upper = shift ^ caps;
        return Some(if upper { c.to_ascii_uppercase() as u8 } else { c as u8 });
    }

    // 数字行: 受 Shift 影响
    if (0x1E..=0x27).contains(&key) {
        let (base, shifted) = match key {
            KEY_1 => (b'1', b'!'),
            KEY_2 => (b'2', b'@'),
            KEY_3 => (b'3', b'#'),
            KEY_4 => (b'4', b'$'),
            KEY_5 => (b'5', b'%'),
            KEY_6 => (b'6', b'^'),
            KEY_7 => (b'7', b'&'),
            KEY_8 => (b'8', b'*'),
            KEY_9 => (b'9', b'('),
            KEY_0 => (b'0', b')'),
            _ => unreachable!(),
        };
        return Some(if shift { shifted } else { base });
    }

    // 符号键
    match key {
        KEY_MINUS      => Some(if shift { b'_' } else { b'-' }),
        KEY_EQUAL      => Some(if shift { b'+' } else { b'=' }),
        KEY_LEFTBRACE  => Some(if shift { b'{' } else { b'[' }),
        KEY_RIGHTBRACE => Some(if shift { b'}' } else { b']' }),
        KEY_BACKSLASH  => Some(if shift { b'|' } else { b'\\' }),
        KEY_SEMICOLON  => Some(if shift { b':' } else { b';' }),
        KEY_APOSTROPHE => Some(if shift { b'"' } else { b'\'' }),
        KEY_GRAVE      => Some(if shift { b'~' } else { b'`' }),
        KEY_COMMA      => Some(if shift { b'<' } else { b',' }),
        KEY_DOT        => Some(if shift { b'>' } else { b'.' }),
        KEY_SLASH      => Some(if shift { b'?' } else { b'/' }),
        KEY_SPACE      => Some(b' '),
        KEY_TAB        => Some(b'\t'),
        KEY_ENTER      => Some(b'\n'),
        KEY_BACKSPACE  => Some(b'\x7f'),
        KEY_ESCAPE     => Some(b'\x1b'),
        _ => None,
    }
}

/// 获取键码的可读名称 (调试输出用)
pub fn keycode_name(key: KeyCode) -> &'static str {
    match key {
        KEY_A..=KEY_Z => {
            let idx = (key - KEY_A) as usize;
            static NAMES: [&str; 26] = [
                "A","B","C","D","E","F","G","H","I","J","K","L","M",
                "N","O","P","Q","R","S","T","U","V","W","X","Y","Z"
            ];
            NAMES[idx]
        }
        KEY_1 => "1", KEY_2 => "2", KEY_3 => "3", KEY_4 => "4", KEY_5 => "5",
        KEY_6 => "6", KEY_7 => "7", KEY_8 => "8", KEY_9 => "9", KEY_0 => "0",
        KEY_ENTER => "Enter", KEY_ESCAPE => "Escape", KEY_BACKSPACE => "Backspace",
        KEY_TAB => "Tab", KEY_SPACE => "Space",
        KEY_CAPSLOCK => "CapsLock",
        KEY_F1..=KEY_F12 => {
            let idx = (key - KEY_F1) as usize + 1;
            match idx { 1 => "F1",2=>"F2",3=>"F3",4=>"F4",5=>"F5",6=>"F6",
                        7=>"F7",8=>"F8",9=>"F9",10=>"F10",11=>"F11",12=>"F12",
                        _ => "F?" }
        }
        KEY_LEFT_CTRL => "L-Ctrl", KEY_LEFT_SHIFT => "L-Shift",
        KEY_LEFT_ALT => "L-Alt", KEY_LEFT_META => "L-Meta",
        KEY_RIGHT_CTRL => "R-Ctrl", KEY_RIGHT_SHIFT => "R-Shift",
        KEY_RIGHT_ALT => "R-Alt", KEY_RIGHT_META => "R-Meta",
        KEY_UP => "Up", KEY_DOWN => "Down", KEY_LEFT => "Left", KEY_RIGHT => "Right",
        KEY_HOME => "Home", KEY_END => "End", KEY_PAGEUP => "PgUp", KEY_PAGEDOWN => "PgDn",
        KEY_INSERT => "Insert", KEY_DELETE => "Delete",
        KEY_MENU => "Menu", KEY_SYSRQ => "SysRq",
        _ => "?"
    }
}
