// tty/mod.rs — TTY/PTY 终端子系统 (POSIX 终端接口)
// 行规则、termios、作业控制、伪终端对
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod ldisc; pub mod termios; pub mod pty; pub mod console;

use crate::sync::SpinLock;
#[repr(C)] pub struct TtyDevice {
    pub name: [u8; 32], pub index: u16, pub flags: u32, pub winsize: WinSize,
    pub termios: Termios, pub ops: &'static TtyOps, pub priv_data: *mut (),
}
#[repr(C)] pub struct WinSize { pub ws_row: u16, pub ws_col: u16, pub ws_xpixel: u16, pub ws_ypixel: u16 }
#[repr(C)] pub struct Termios { pub c_iflag: u32, pub c_oflag: u32, pub c_cflag: u32, pub c_lflag: u32, pub c_cc: [u8; 32] }
#[repr(C)] pub struct TtyOps {
    pub write: unsafe extern "C" fn(tty: *mut TtyDevice, buf: *const u8, len: usize) -> isize,
    pub read: unsafe extern "C" fn(tty: *mut TtyDevice, buf: *mut u8, len: usize) -> isize,
    pub ioctl: unsafe extern "C" fn(tty: *mut TtyDevice, cmd: u32, arg: *mut ()) -> i32,
}
pub struct TtyManager { pub ttys: [Option<*mut TtyDevice>; 64], pub count: usize }
unsafe impl Send for TtyManager {}
static TTY_MGR: SpinLock<TtyManager> = SpinLock::new(TtyManager { ttys: [None; 64], count: 0 });
pub fn init() { pty::init(); console::init_tty();
    crate::serial::write_str(b"tty: subsystem ready\n"); }

/// 会话管理周期任务（sessiond 调用）
pub fn session_tick() {
    let _mgr = TTY_MGR.lock();
}
