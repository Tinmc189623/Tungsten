// devices/input/mod.rs — Tungsten 输入子系统核心
//
// 统一事件队列 + 驱动注册 + IRQ 路由 + 多键盘支持 + LED 同步 + SysRq
// 所有输入设备 (PS/2、USB HID) 将事件推入队列，上层消费。
//
// 架构:
//   驱动层 -> push_event() -> [环形事件缓冲] -> read_event() -> 上层
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod keycode;
pub mod ps2;
pub mod usb;
pub mod layout;

use keycode::KeyCode;
use crate::sync::IrqSaveSpinlock;

/* ── 输入事件 ── */

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    KeyPress = 0,
    KeyRelease = 1,
}

/// 输入事件结构 (栈上值类型，无堆分配)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub event_type: EventType,
    pub key: KeyCode,
    /// 修饰键掩码 (MOD_LCTRL | MOD_LSHIFT 等)
    pub modifiers: u8,
    /// 时间戳 (APIC 定时器 tick 数)
    pub timestamp: u64,
    /// 设备 ID (区分多键盘: 0=PS/2, 1+=USB HID)
    pub device_id: u8,
}

impl InputEvent {
    /// 创建输入事件 (默认 PS/2 设备)
    pub const fn new(event_type: EventType, key: KeyCode, modifiers: u8) -> Self {
        InputEvent { event_type, key, modifiers, timestamp: 0, device_id: 0 }
    }

    /// 创建指定设备 ID 的输入事件
    pub fn with_devid(event_type: EventType, key: KeyCode, modifiers: u8, device_id: u8) -> Self {
        InputEvent { event_type, key, modifiers, timestamp: 0, device_id }
    }
}

/* ── 事件队列 ── */

const EVENT_QUEUE_SIZE: usize = 256;

/// 环形事件缓冲区
struct EventQueue {
    buffer: [InputEvent; EVENT_QUEUE_SIZE],
    head: usize,
    tail: usize,
    count: usize,
}

impl EventQueue {
    /// 创建空事件队列
    const fn new() -> Self {
        EventQueue {
            buffer: [InputEvent::new(EventType::KeyPress, 0, 0); EVENT_QUEUE_SIZE],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// 推入事件 (队列满时返回 false)
    fn push(&mut self, ev: InputEvent) -> bool {
        if self.count >= EVENT_QUEUE_SIZE {
            return false;
        }
        self.buffer[self.tail] = ev;
        self.tail = (self.tail + 1) % EVENT_QUEUE_SIZE;
        self.count += 1;
        true
    }

    /// 弹出最早的事件
    fn pop(&mut self) -> Option<InputEvent> {
        if self.count == 0 {
            return None;
        }
        let ev = self.buffer[self.head];
        self.head = (self.head + 1) % EVENT_QUEUE_SIZE;
        self.count -= 1;
        Some(ev)
    }

    /// 清空队列
    fn flush(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }

    /// 队列是否为空
    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/* ── 全局状态 ── */

static EVENT_QUEUE: IrqSaveSpinlock<EventQueue> = IrqSaveSpinlock::new(EventQueue::new());

/// 当前修饰键状态 (全局跟踪)
static MODIFIER_STATE: IrqSaveSpinlock<u8> = IrqSaveSpinlock::new(0);

/// 当前大写锁定状态
static CAPS_LOCK_STATE: IrqSaveSpinlock<bool> = IrqSaveSpinlock::new(false);

/// 已登记的键盘设备数量
static KEYBOARD_DEVICE_COUNT: IrqSaveSpinlock<u8> = IrqSaveSpinlock::new(0);

/// 主力输入设备 ID (0xff = 未定, 0=PS/2, 1+=USB)
static ACTIVE_KB_DEVICE: IrqSaveSpinlock<u8> = IrqSaveSpinlock::new(0xFF);

/* ── SysRq 框架 ── */

/// SysRq 是否已触发 (Alt+SysRq 组合键)
static SYSRQ_ACTIVE: IrqSaveSpinlock<bool> = IrqSaveSpinlock::new(false);

/// 处理 SysRq 命令
fn handle_sysrq(key: KeyCode) {
    use keycode::*;
    match key {
        KEY_H | KEY_HELP => {
            crate::serial::write_str(b"\n=== SysRq Help ===\n");
            crate::serial::write_str(b"  h/Help  - this help\n");
            crate::serial::write_str(b"  b       - reboot\n");
            crate::serial::write_str(b"  p       - dump PCI devices\n");
            crate::serial::write_str(b"  t       - dump tasks\n");
            crate::serial::write_str(b"  k       - SAK (Secure Attention Key)\n");
        }
        KEY_B => {
            crate::serial::write_str(b"\n[SysRq] reboot\n");
            unsafe {
                core::arch::asm!("out 0x64, al", in("al") 0xFEu8);
            }
        }
        KEY_P => {
            crate::serial::write_str(b"\n[SysRq] PCI devices:\n");
            let count = crate::devices::pci::device_count();
            for i in 0..count {
                let d = crate::devices::pci::devices()[i];
                crate::serial::write_str(b"  ");
                crate::serial_put_u64(d.bus as u64);
                crate::serial::write_str(b":");
                crate::serial_put_u64(d.dev as u64);
                crate::serial::write_str(b".");
                crate::serial_put_u64(d.func as u64);
                crate::serial::write_str(b" class=");
                crate::serial_put_u64(d.class_code as u64);
                crate::serial::write_str(b"/");
                crate::serial_put_u64(d.subclass as u64);
                crate::serial::write_str(b"\n");
            }
        }
        KEY_T => {
            crate::serial::write_str(b"\n[SysRq] task info:\n");
            crate::serial::write_str(b"  current tid: ");
            crate::serial_put_u64(crate::sched::current_tid());
            crate::serial::write_str(b"\n");
        }
        KEY_K => {
            crate::serial::write_str(b"\n[SysRq] SAK: flushing event queue\n");
            EVENT_QUEUE.lock().flush();
        }
        _ => {}
    }
}

/* ── 输入状态跟踪 ── */

/// 键盘状态跟踪器 (修饰键 + 锁键)
pub struct KeyboardState {
    pub modifiers: u8,
    pub caps: bool,
    pub scroll: bool,
    pub num: bool,
}

impl KeyboardState {
    /// 创建初始键盘状态
    pub const fn new() -> Self {
        KeyboardState { modifiers: 0, caps: false, scroll: false, num: false }
    }

    /// 根据按键事件更新修饰键和锁键状态
    pub fn update_modifier(&mut self, key: KeyCode, pressed: bool) {
        let bit = match key {
            keycode::KEY_LEFT_CTRL  | keycode::KEY_RIGHT_CTRL  => Some(keycode::MOD_LCTRL | keycode::MOD_RCTRL),
            keycode::KEY_LEFT_SHIFT | keycode::KEY_RIGHT_SHIFT => Some(keycode::MOD_LSHIFT | keycode::MOD_RSHIFT),
            keycode::KEY_LEFT_ALT   | keycode::KEY_RIGHT_ALT   => Some(keycode::MOD_LALT | keycode::MOD_RALT),
            keycode::KEY_LEFT_META  | keycode::KEY_RIGHT_META  => Some(keycode::MOD_LMETA | keycode::MOD_RMETA),
            _ => None,
        };
        if let Some(mask) = bit {
            if pressed { self.modifiers |= mask; } else { self.modifiers &= !mask; }
        }
        if pressed {
            match key {
                keycode::KEY_CAPSLOCK => { self.caps = !self.caps; }
                keycode::KEY_SCROLLLOCK => { self.scroll = !self.scroll; }
                keycode::KEY_KP_NUMLOCK => { self.num = !self.num; }
                _ => {}
            }
        }
    }
}

/* ── 公共 API ── */

/// 驱动调用: 推送输入事件 (可在中断上下文调用)
pub fn push_event(ev: InputEvent) {
    EVENT_QUEUE.lock().push(ev);
}

/// 读取一个输入事件
pub fn read_event() -> Option<InputEvent> {
    EVENT_QUEUE.lock().pop()
}

/// 检查是否有待处理事件
pub fn has_event() -> bool {
    EVENT_QUEUE.lock().is_empty() == false
}

/// 获取当前修饰键状态
pub fn modifier_state() -> u8 { *MODIFIER_STATE.lock() }

/// 获取大写锁定状态
pub fn caps_lock_state() -> bool { *CAPS_LOCK_STATE.lock() }

/// 设置修饰键状态
pub fn set_modifier_state(mods: u8) { *MODIFIER_STATE.lock() = mods; }

/// 设置大写锁定状态
pub fn set_caps_lock_state(caps: bool) { *CAPS_LOCK_STATE.lock() = caps; }

/// 登记一个键盘设备，返回 device_id
pub fn register_keyboard() -> u8 {
    let id = 0u8;
    // 第一个登记的设备成为主力
    crate::serial::write_str(b"input: keyboard dev#");
    crate::serial_put_u64(id as u64);
    crate::serial::write_str(b" registered\n");
    id
}

/// 获取活跃键盘设备数
pub fn keyboard_count() -> u8 {
    *KEYBOARD_DEVICE_COUNT.lock()
}

/// 设置主力输入设备
pub fn set_active_keyboard(dev_id: u8) {
    *ACTIVE_KB_DEVICE.lock() = dev_id;
    crate::serial::write_str(b"input: active keyboard switched to dev#");
    crate::serial_put_u64(dev_id as u64);
    crate::serial::write_str(b"\n");
}

/// 获取主力输入设备 ID
pub fn active_keyboard() -> u8 {
    *ACTIVE_KB_DEVICE.lock()
}

/// 检查 SysRq 是否被触发
pub fn sysrq_active() -> bool {
    *SYSRQ_ACTIVE.lock()
}

/// 处理一次输入事件 (供消费方循环调用)
/// 返回 true 表示事件已消费
pub fn process_event(ev: &InputEvent) -> bool {
    // SysRq: Alt + SysRq + key
    let mods = ev.modifiers;
    let alt_pressed = (mods & (keycode::MOD_LALT | keycode::MOD_RALT)) != 0;

    if ev.event_type == EventType::KeyPress && alt_pressed && ev.key == keycode::KEY_SYSRQ {
        *SYSRQ_ACTIVE.lock() = true;
        crate::serial::write_str(b"[SysRq] activated\n");
        return true;
    }

    // SysRq 命令键
    if sysrq_active() && ev.event_type == EventType::KeyPress {
        handle_sysrq(ev.key);
        *SYSRQ_ACTIVE.lock() = false;
        return true;
    }

    // 释放 SysRq 键时取消 SysRq 模式
    if ev.event_type == EventType::KeyRelease && ev.key == keycode::KEY_SYSRQ {
        *SYSRQ_ACTIVE.lock() = false;
    }

    false
}

/// 同步 LED 到 PS/2 键盘
pub fn sync_leds() {
    let caps = *CAPS_LOCK_STATE.lock();
    ps2::set_leds(caps, false, false);
}

/* ── 初始化 ── */

/// 初始化输入子系统 (PS/2 + USB HID)
pub fn init() {
    // PS/2 键盘初始化 (legacy fallback)
    ps2::init();

    crate::serial::write_str(b"input: input subsystem initialized\n");

    // USB HID 键盘初始化 (主力输入)
    crate::serial::write_str(b"input: initializing USB HID...\n");
    usb::init();

    if usb::keyboard_available() {
        crate::serial::write_str(b"input: USB HID keyboard ready (primary)\n");
    } else {
        crate::serial::write_str(b"input: USB HID keyboard not found, using PS/2\n");
    }

    // 首次 LED 同步
    sync_leds();
}
