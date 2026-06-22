// devices/input/ps2.rs — PS/2 键盘驱动 (legacy fallback)
//
// 使用 I/O 端口 0x60/0x64 访问 PS/2 控制器。
// 默认使用 Scancode Set 2，翻译为统一键码后推入输入事件队列。
//
// 架构:
//   IRQ 1 (vec 33) -> ps2_irq_handler()
//                   -> ps2_process_scancode(byte)
//                   -> push_event(InputEvent)
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later


use crate::serial;
use crate::devices::input::{self, EventType, InputEvent};
use crate::devices::input::keycode;
use crate::devices::input::KeyboardState;

/* ── PS/2 I/O 端口 ── */

const PS2_DATA: u16    = 0x60;
const PS2_STATUS: u16  = 0x64;
const PS2_CMD: u16     = 0x64;

/* ── PS/2 状态寄存器位 ── */

const STAT_OUTPUT_FULL:  u8 = 0x01;
const STAT_INPUT_FULL:   u8 = 0x02;
const STAT_SYSTEM_FLAG:  u8 = 0x04;
const STAT_CMD_DATA:     u8 = 0x08;
const STAT_TIMEOUT:      u8 = 0x60;
const STAT_PARITY_ERR:   u8 = 0x80;

/* ── PS/2 控制器命令 ── */

const CMD_READ_CCB:      u8 = 0x20;
const CMD_WRITE_CCB:     u8 = 0x60;
const CMD_DISABLE_PORT1: u8 = 0xAD;
const CMD_ENABLE_PORT1:  u8 = 0xAE;
const CMD_DISABLE_PORT2: u8 = 0xA7;
const CMD_ENABLE_PORT2:  u8 = 0xA8;
const CMD_TEST_PORT1:    u8 = 0xAB;
const CMD_TEST_CONTROLLER: u8 = 0xAA;

/* ── 键盘命令 ── */

const KB_CMD_RESET:          u8 = 0xFF;
const KB_CMD_SET_SCANSET:    u8 = 0xF0;
const KB_CMD_SET_LEDS:       u8 = 0xED;
const KB_CMD_TYPEMATIC:      u8 = 0xF3;
const KB_CMD_ENABLE_SCAN:    u8 = 0xF4;
const KB_CMD_DISABLE_SCAN:   u8 = 0xF5;

/* ── 键盘响应 ── */

const KB_ACK:   u8 = 0xFA;
const KB_RESEND: u8 = 0xFE;
const KB_BAT:   u8 = 0xAA;

/* ── 外部 HAL 函数 ── */

#[link(name = "hal_tungsten", kind = "static")]
unsafe extern "C" {
    fn hal_inb(port: u16) -> u8;
    fn hal_outb(port: u16, val: u8);
}

/* ── 等待函数 ── */

/// 等待直到输出缓冲有数据 (最多 10000 次)
fn wait_output() -> bool {
    for _ in 0..10000 {
        let status = unsafe { hal_inb(PS2_STATUS) };
        if status & STAT_OUTPUT_FULL != 0 {
            return true;
        }
        unsafe { core::arch::asm!("pause") };
    }
    false
}

/// 等待直到输入缓冲为空 (最多 10000 次)
fn wait_input() -> bool {
    for _ in 0..10000 {
        let status = unsafe { hal_inb(PS2_STATUS) };
        if status & STAT_INPUT_FULL == 0 {
            return true;
        }
        unsafe { core::arch::asm!("pause") };
    }
    false
}

/// 从 PS/2 数据端口读一个字节 (忙等待)
unsafe fn ps2_read() -> u8 {
    while (hal_inb(PS2_STATUS) & STAT_OUTPUT_FULL) == 0 {
        core::arch::asm!("pause");
    }
    hal_inb(PS2_DATA)
}

/// 向 PS/2 数据端口写一个字节 (忙等待)
unsafe fn ps2_write(val: u8) {
    while (hal_inb(PS2_STATUS) & STAT_INPUT_FULL) != 0 {
        core::arch::asm!("pause");
    }
    hal_outb(PS2_DATA, val);
}

/// 向 PS/2 命令端口写一个字节
unsafe fn ps2_cmd(cmd: u8) {
    while (hal_inb(PS2_STATUS) & STAT_INPUT_FULL) != 0 {
        core::arch::asm!("pause");
    }
    hal_outb(PS2_CMD, cmd);
}

/// 向键盘发送命令并等待 ACK (最多重试 3 次)
unsafe fn keyboard_send(cmd: u8) -> bool {
    let mut retries = 3;
    while retries > 0 {
        ps2_write(cmd);
        let resp = ps2_read();
        if resp == KB_ACK {
            return true;
        }
        if resp != KB_RESEND {
            break;
        }
        retries -= 1;
    }
    false
}

/* ── 驱动状态 ── */

/// PS/2 控制器是否存在
static mut PS2_PRESENT: bool = false;

/// 当前使用的扫描码集 (默认 Set 2)
static mut SCANSET: u8 = 2;

/// PS/2 键盘状态跟踪器
static mut KEY_STATE: KeyboardState = KeyboardState::new();

/* ── Scancode Set 1 翻译表 ── */

/// Set 1 Make 码 -> 键码表 (索引 = scancode & 0x7F)
static SCANSET1_TABLE: [keycode::KeyCode; 128] = [
    keycode::KEY_NONE, keycode::KEY_ESCAPE, keycode::KEY_1, keycode::KEY_2,
    keycode::KEY_3, keycode::KEY_4, keycode::KEY_5, keycode::KEY_6,
    keycode::KEY_7, keycode::KEY_8, keycode::KEY_9, keycode::KEY_0,
    keycode::KEY_MINUS, keycode::KEY_EQUAL, keycode::KEY_BACKSPACE, keycode::KEY_TAB,
    keycode::KEY_Q, keycode::KEY_W, keycode::KEY_E, keycode::KEY_R,
    keycode::KEY_T, keycode::KEY_Y, keycode::KEY_U, keycode::KEY_I,
    keycode::KEY_O, keycode::KEY_P, keycode::KEY_LEFTBRACE, keycode::KEY_RIGHTBRACE,
    keycode::KEY_ENTER, keycode::KEY_LEFT_CTRL, keycode::KEY_A, keycode::KEY_S,
    keycode::KEY_D, keycode::KEY_F, keycode::KEY_G, keycode::KEY_H,
    keycode::KEY_J, keycode::KEY_K, keycode::KEY_L, keycode::KEY_SEMICOLON,
    keycode::KEY_APOSTROPHE, keycode::KEY_GRAVE, keycode::KEY_LEFT_SHIFT, keycode::KEY_BACKSLASH,
    keycode::KEY_Z, keycode::KEY_X, keycode::KEY_C, keycode::KEY_V,
    keycode::KEY_B, keycode::KEY_N, keycode::KEY_M, keycode::KEY_COMMA,
    keycode::KEY_DOT, keycode::KEY_SLASH, keycode::KEY_RIGHT_SHIFT, keycode::KEY_KP_ASTERISK,
    keycode::KEY_LEFT_ALT, keycode::KEY_SPACE, keycode::KEY_CAPSLOCK,
    keycode::KEY_F1, keycode::KEY_F2, keycode::KEY_F3, keycode::KEY_F4, 0,
    keycode::KEY_F5, keycode::KEY_F6, keycode::KEY_F7, keycode::KEY_F8,
    keycode::KEY_F9, keycode::KEY_F10, keycode::KEY_KP_NUMLOCK, keycode::KEY_SCROLLLOCK,
    keycode::KEY_KP_7, keycode::KEY_KP_8, keycode::KEY_KP_9, keycode::KEY_KP_MINUS,
    keycode::KEY_KP_4, keycode::KEY_KP_5, keycode::KEY_KP_6, keycode::KEY_KP_PLUS,
    keycode::KEY_KP_1, keycode::KEY_KP_2, keycode::KEY_KP_3, keycode::KEY_KP_0,
    keycode::KEY_KP_DOT, 0, 0, keycode::KEY_F11,
    keycode::KEY_F12, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];

/// Set 1 0xE0 前缀扩展码 -> 键码
static SCANSET1_E0: [(u8, keycode::KeyCode); 16] = [
    (0x1C, keycode::KEY_KP_ENTER),
    (0x1D, keycode::KEY_RIGHT_CTRL),
    (0x35, keycode::KEY_KP_SLASH),
    (0x38, keycode::KEY_RIGHT_ALT),
    (0x47, keycode::KEY_HOME),
    (0x48, keycode::KEY_UP),
    (0x49, keycode::KEY_PAGEUP),
    (0x4B, keycode::KEY_LEFT),
    (0x4D, keycode::KEY_RIGHT),
    (0x4F, keycode::KEY_END),
    (0x50, keycode::KEY_DOWN),
    (0x51, keycode::KEY_PAGEDOWN),
    (0x52, keycode::KEY_INSERT),
    (0x53, keycode::KEY_DELETE),
    (0x5B, keycode::KEY_LEFT_META),
    (0x5C, keycode::KEY_RIGHT_META),
];

/* ── Scancode Set 2 翻译表 ── */

static SCANSET2_TABLE: [keycode::KeyCode; 128] = [
    keycode::KEY_NONE, keycode::KEY_F9, keycode::KEY_NONE, keycode::KEY_F5,
    keycode::KEY_F3, keycode::KEY_F1, keycode::KEY_F2, keycode::KEY_F12,
    keycode::KEY_NONE, keycode::KEY_F10, keycode::KEY_F8, keycode::KEY_F6,
    keycode::KEY_F4, keycode::KEY_TAB, keycode::KEY_GRAVE, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_LEFT_ALT, keycode::KEY_LEFT_SHIFT, keycode::KEY_NONE,
    keycode::KEY_LEFT_CTRL, keycode::KEY_Q, keycode::KEY_1, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_Z, keycode::KEY_S,
    keycode::KEY_A, keycode::KEY_W, keycode::KEY_2, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_C, keycode::KEY_X, keycode::KEY_D,
    keycode::KEY_E, keycode::KEY_4, keycode::KEY_3, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_SPACE, keycode::KEY_V, keycode::KEY_F,
    keycode::KEY_T, keycode::KEY_R, keycode::KEY_5, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_N, keycode::KEY_B, keycode::KEY_H,
    keycode::KEY_G, keycode::KEY_Y, keycode::KEY_6, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_M, keycode::KEY_J,
    keycode::KEY_U, keycode::KEY_7, keycode::KEY_8, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_COMMA, keycode::KEY_K, keycode::KEY_I,
    keycode::KEY_O, keycode::KEY_0, keycode::KEY_9, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_DOT, keycode::KEY_SLASH, keycode::KEY_L,
    keycode::KEY_SEMICOLON, keycode::KEY_P, keycode::KEY_MINUS, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_APOSTROPHE, keycode::KEY_NONE,
    keycode::KEY_LEFTBRACE, keycode::KEY_EQUAL, keycode::KEY_NONE, keycode::KEY_NONE,
    keycode::KEY_CAPSLOCK, keycode::KEY_RIGHT_SHIFT, keycode::KEY_ENTER, keycode::KEY_RIGHTBRACE,
    keycode::KEY_NONE, keycode::KEY_BACKSLASH, keycode::KEY_NONE, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_BACKSPACE, keycode::KEY_NONE,
    keycode::KEY_NONE, keycode::KEY_KP_1, keycode::KEY_NONE, keycode::KEY_KP_4,
    keycode::KEY_KP_7, keycode::KEY_NONE, keycode::KEY_NONE, keycode::KEY_NONE,
    keycode::KEY_KP_0, keycode::KEY_KP_DOT, keycode::KEY_KP_2, keycode::KEY_KP_5,
    keycode::KEY_KP_6, keycode::KEY_KP_8, keycode::KEY_ESCAPE, keycode::KEY_KP_NUMLOCK,
    keycode::KEY_F11, keycode::KEY_KP_PLUS, keycode::KEY_KP_3, keycode::KEY_KP_MINUS,
    keycode::KEY_KP_ASTERISK, keycode::KEY_KP_9, keycode::KEY_SCROLLLOCK, keycode::KEY_NONE,
];

/// Set 2 0xE0 前缀扩展码
static SCANSET2_E0: [(u8, keycode::KeyCode); 17] = [
    (0x11, keycode::KEY_RIGHT_ALT),
    (0x12, keycode::KEY_LEFT_META),
    (0x14, keycode::KEY_RIGHT_CTRL),
    (0x1F, keycode::KEY_LEFT_META),
    (0x27, keycode::KEY_RIGHT_META),
    (0x2F, keycode::KEY_MENU),
    (0x4A, keycode::KEY_KP_SLASH),
    (0x5A, keycode::KEY_KP_ENTER),
    (0x5E, keycode::KEY_POWER),
    (0x69, keycode::KEY_END),
    (0x6B, keycode::KEY_LEFT),
    (0x6C, keycode::KEY_HOME),
    (0x70, keycode::KEY_INSERT),
    (0x71, keycode::KEY_DELETE),
    (0x72, keycode::KEY_DOWN),
    (0x74, keycode::KEY_RIGHT),
    (0x75, keycode::KEY_UP),
];

/* ── 扫描码处理 ── */

/// PS/2 解析状态机状态
enum Ps2ParseState {
    Normal,
    E0Prefix,
    E1Prefix,
}

/// PS/2 键盘解析器
struct Ps2Parser {
    state: Ps2ParseState,
    key_state: KeyboardState,
    scancode_set: u8,
}

impl Ps2Parser {
    /// 创建新的 PS/2 解析器 (默认 Set 2)
    const fn new() -> Self {
        Ps2Parser {
            state: Ps2ParseState::Normal,
            key_state: KeyboardState::new(),
            scancode_set: 2,
        }
    }

    /// 喂入一个原始字节，产生 0 或 1 个输入事件
    fn feed_byte(&mut self, byte: u8) -> Option<InputEvent> {
        match self.state {
            Ps2ParseState::Normal => {
                match byte {
                    0xE0 => {
                        self.state = Ps2ParseState::E0Prefix;
                        None
                    }
                    0xE1 => {
                        self.state = Ps2ParseState::E1Prefix;
                        None
                    }
                    _ => {
                        self.process_scancode(byte)
                    }
                }
            }
            Ps2ParseState::E0Prefix => {
                self.state = Ps2ParseState::Normal;
                self.process_e0_scancode(byte)
            }
            Ps2ParseState::E1Prefix => {
                // Pause 键: E1 14 77 E1 F0 14 F0 77
                self.state = Ps2ParseState::Normal;
                None
            }
        }
    }

    /// 处理普通扫描码 (无前缀)
    fn process_scancode(&mut self, byte: u8) -> Option<InputEvent> {
        let is_break = if self.scancode_set == 2 {
            false // Set 2 的 Break 由 0xF0 前缀处理
        } else {
            byte & 0x80 != 0
        };

        if self.scancode_set == 2 {
            let scancode = byte & 0x7F;
            if scancode >= 128 { return None; }
            let key = SCANSET2_TABLE[scancode as usize];
            if key == keycode::KEY_NONE { return None; }

            self.key_state.update_modifier(key, true);
            let mods = self.key_state.modifiers;
            Some(InputEvent::new(EventType::KeyPress, key, mods))
        } else {
            let scancode = (byte & 0x7F) as usize;
            if scancode >= 128 { return None; }
            let key = SCANSET1_TABLE[scancode];
            if key == keycode::KEY_NONE { return None; }

            self.key_state.update_modifier(key, !is_break);
            let mods = self.key_state.modifiers;
            let ev_type = if is_break { EventType::KeyRelease } else { EventType::KeyPress };
            Some(InputEvent::new(ev_type, key, mods))
        }
    }

    /// 处理 0xE0 前缀扩展扫描码
    fn process_e0_scancode(&mut self, byte: u8) -> Option<InputEvent> {
        let is_break = if self.scancode_set == 2 {
            false
        } else {
            byte & 0x80 != 0
        };

        let scancode = byte & 0x7F;
        let key = if self.scancode_set == 2 {
            SCANSET2_E0.iter().find(|(s, _)| *s == scancode).map(|(_, k)| *k)
        } else {
            SCANSET1_E0.iter().find(|(s, _)| *s == scancode).map(|(_, k)| *k)
        }.unwrap_or(keycode::KEY_NONE);

        if key == keycode::KEY_NONE { return None; }

        let pressed = !is_break;
        self.key_state.update_modifier(key, pressed);
        let mods = self.key_state.modifiers;
        let ev_type = if pressed { EventType::KeyPress } else { EventType::KeyRelease };
        Some(InputEvent::new(ev_type, key, mods))
    }
}

/* ── 扫描码解码器 (处理 Set 2 的 0xF0 Break 前缀) ── */

struct Ps2Decoder {
    parser: Ps2Parser,
    pending_f0: bool,
    pending_e0: bool,
    pending_e1: bool,
    e1_count: u8,
}

impl Ps2Decoder {
    /// 创建新的解码器
    const fn new() -> Self {
        Ps2Decoder {
            parser: Ps2Parser::new(),
            pending_f0: false,
            pending_e0: false,
            pending_e1: false,
            e1_count: 0,
        }
    }

    /// 喂入一个原始字节，产生 0 或 1 个输入事件
    fn feed(&mut self, byte: u8) -> Option<InputEvent> {
        // 处理 Set 2 的 0xE1 (Pause 键序列)
        if self.pending_e1 {
            self.e1_count += 1;
            if self.e1_count >= 8 {
                self.pending_e1 = false;
                self.e1_count = 0;
            }
            return None;
        }

        if byte == 0xE1 {
            self.pending_e1 = true;
            self.e1_count = 1;
            return None;
        }

        // Set 2: 0xF0 是 Break 前缀
        if self.parser.scancode_set == 2 {
            if byte == 0xF0 {
                self.pending_f0 = true;
                return None;
            }

            if byte == 0xE0 {
                self.pending_e0 = true;
                return None;
            }

            let is_break = self.pending_f0;
            let is_extended = self.pending_e0;
            self.pending_f0 = false;
            self.pending_e0 = false;

            let scancode = byte & 0x7F;
            if scancode >= 128 { return None; }

            let key = if is_extended {
                SCANSET2_E0.iter().find(|(s, _)| *s == scancode).map(|(_, k)| *k)
                    .unwrap_or(keycode::KEY_NONE)
            } else {
                SCANSET2_TABLE[scancode as usize]
            };

            if key == keycode::KEY_NONE { return None; }

            let pressed = !is_break;
            self.parser.key_state.update_modifier(key, pressed);
            let mods = self.parser.key_state.modifiers;
            let ev_type = if pressed { EventType::KeyPress } else { EventType::KeyRelease };

            // 大写锁定同步到全局
            if pressed && key == keycode::KEY_CAPSLOCK {
                input::set_caps_lock_state(self.parser.key_state.caps);
            }
            input::set_modifier_state(mods);

            Some(InputEvent::new(ev_type, key, mods))
        } else {
            // Set 1: 标准处理
            self.parser.feed_byte(byte)
        }
    }
}

/// 全局 PS/2 解码器
static mut PS2_DECODER: Ps2Decoder = Ps2Decoder::new();

/* ── PS/2 初始化 ── */

/// 测试 PS/2 控制器是否存在
fn probe_controller() -> bool {
    unsafe {
        ps2_cmd(CMD_DISABLE_PORT1);
        ps2_cmd(CMD_READ_CCB);
        let ccb = ps2_read();
        ps2_cmd(CMD_WRITE_CCB);
        ps2_write(ccb | 0x01);

        ps2_cmd(CMD_TEST_CONTROLLER);
        let test = ps2_read();
        test == 0x55
    }
}

/// 初始化 PS/2 键盘 (探测控制器 -> 复位 -> 启用扫描)
pub fn init() {
    unsafe {
        serial::write_str(b"ps2: probing PS/2 controller...\n");

        if !probe_controller() {
            serial::write_str(b"ps2: controller not found\n");
            PS2_PRESENT = false;
            return;
        }
        PS2_PRESENT = true;

        serial::write_str(b"ps2: controller found\n");

        // 启用键盘接口
        ps2_cmd(CMD_ENABLE_PORT1);

        // 复位键盘
        serial::write_str(b"ps2: resetting keyboard...\n");
        if !keyboard_send(KB_CMD_RESET) {
            serial::write_str(b"ps2: keyboard reset failed\n");
            return;
        }
        // 等待 BAT 完成码
        let bat = ps2_read();
        serial::write_str(b"ps2: BAT response: 0x");
        if bat == KB_BAT {
            serial::write_str(b"AA (OK)\n");
        } else {
            serial::write_str(b"??\n");
        }

        // 使用 Scancode Set 2
        serial::write_str(b"ps2: using scancode set 2\n");
        PS2_DECODER.parser.scancode_set = 2;

        // 启用扫描
        keyboard_send(KB_CMD_ENABLE_SCAN);

        // 清空缓冲
        while (hal_inb(PS2_STATUS) & STAT_OUTPUT_FULL) != 0 {
            hal_inb(PS2_DATA);
        }

        serial::write_str(b"ps2: keyboard ready (IRQ 1)\n");
    }

    // 登记键盘设备 (device_id = 0, PS/2)
    crate::devices::input::register_keyboard();
}

/* ── IRQ 处理 ── */

/// 处理一次 PS/2 中断 (主动轮询替代)
pub fn poll() {
    unsafe {
        if !PS2_PRESENT { return; }
        let status = hal_inb(PS2_STATUS);
        if status & STAT_OUTPUT_FULL != 0 {
            let byte = hal_inb(PS2_DATA);
            let decoder = &raw mut PS2_DECODER;
            if let Some(ev) = (*decoder).feed(byte) {
                input::push_event(ev);
            }
        }
    }
}

/// IRQ 1 中断处理入口
pub fn irq_handler() {
    poll();
}

/* ── 主动轮询 ── */

/// 持续轮询 PS/2 键盘 (最多处理 64 个事件)
/// 在串口 shell 循环中调用
pub fn drain() {
    for _ in 0..64 {
        poll();
        if !unsafe { (hal_inb(PS2_STATUS) & STAT_OUTPUT_FULL) != 0 } {
            break;
        }
    }
}

/* ── LED 控制 ── */

/// 设置 PS/2 键盘 LED (CapsLock / NumLock / ScrollLock)
/// 通过向键盘发送 0xED 命令 + LED 状态字节
pub fn set_leds(caps: bool, num: bool, scroll: bool) {
    unsafe {
        if !PS2_PRESENT { return; }

        // LED 状态字节: bit0=ScrollLock, bit1=NumLock, bit2=CapsLock
        let mut led_val: u8 = 0;
        if scroll { led_val |= 1; }
        if num    { led_val |= 2; }
        if caps   { led_val |= 4; }

        let mut retries = 3;
        while retries > 0 {
            ps2_write(KB_CMD_SET_LEDS);
            let ack = ps2_read();
            if ack == KB_ACK {
                ps2_write(led_val);
                break;
            }
            retries -= 1;
        }
    }
}
