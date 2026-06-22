// backtrace.rs — 内核堆栈回溯 (x86_64 Frame Pointer)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later
use core::arch::asm;
pub const MAX_FRAMES: usize = 64;
#[repr(C)] pub struct StackFrame { pub rbp: u64, pub rip: u64 }
#[repr(C)] pub struct Backtrace { pub frames: [u64; MAX_FRAMES], pub count: usize }
static mut LAST_BACKTRACE: Backtrace = Backtrace { frames: [0; MAX_FRAMES], count: 0 };
pub unsafe fn capture() {
    let mut rbp: u64; asm!("mov {}, rbp", out(reg) rbp);
    let bt = &mut *core::ptr::addr_of_mut!(LAST_BACKTRACE);
    bt.count = 0;
    for i in 0..MAX_FRAMES {
        if rbp == 0 || rbp < 0xFFFF800000000000 { break; }
        let frame = rbp as *const StackFrame;
        let rip = core::ptr::read_unaligned(&(*frame).rip);
        let next_rbp = core::ptr::read_unaligned(&(*frame).rbp);
        bt.frames[i] = rip; bt.count = i + 1; rbp = next_rbp;
        if rbp == 0 || rbp < next_rbp { break; }
    }
}
pub fn print_backtrace() {
    let bt = unsafe { &*core::ptr::addr_of!(LAST_BACKTRACE) };
    crate::serial::write_str(b"\n=== STACK BACKTRACE ===\n");
    for i in 0..bt.count { crate::serial::write_str(b"  #");
        crate::serial_put_u64(i as u64); crate::serial::write_str(b"  RIP=0x");
        crate::serial_put_u64_hex(bt.frames[i]); crate::serial::write_str(b"\n"); }
    crate::serial::write_str(b"=== END ===\n");
}
pub fn get_backtrace() -> &'static Backtrace { unsafe { &*core::ptr::addr_of!(LAST_BACKTRACE) } }
