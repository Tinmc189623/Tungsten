// arch/x86_64/syscall.rs — SYSCALL/SYSRET 入口 + MSR 配置
// x86_64 长模式 syscall 指令实现 Ring 3 → Ring 0 特权级切换
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::arch::asm;
use core::arch::naked_asm;

/* ── MSR 地址常量 ── */

/// SYSCALL Target Address Register — STAR[47:32]=syscall CS, STAR[63:48]=sysret CS
const IA32_STAR: u32  = 0xC000_0081;
/// SYSCALL 入口地址（Ring 0 RIP）
const IA32_LSTAR: u32 = 0xC000_0082;
/// SYSCALL RFLAGS 掩码（进入 Ring 0 时清除的位）
const IA32_FMASK: u32 = 0xC000_0084;

/* ── 栈帧 ── */

/// syscall 入口 trampoline 保存的寄存器栈帧
///
/// CPU 自动完成：rcx = user RIP, r11 = user RFLAGS, RSP = TSS.RSP0
/// trampoline 再依次保存其余寄存器到此结构
#[repr(C)]
pub struct SyscallFrame {
    /// r11 — 用户态 RFLAGS
    pub user_rflags: u64,
    /// rcx — 用户态返回地址
    pub user_rip: u64,
    /// rax — 系统调用编号
    pub num: u64,
    /// rdi, rsi, rdx, r10, r8, r9 — 系统调用参数
    pub args: [u64; 6],
}

/* ── 自定义处理函数表 ── */

/// 最大系统调用编号（可扩展）
const MAX_SYSCALL_NR: usize = 512;

/// 自定义 syscall 处理函数类型
type SyscallHandlerFn = fn(frame: &SyscallFrame) -> u64;

/// 自定义处理函数注册表（syscall 编号 → 函数指针）
static mut SYSCALL_HANDLERS: [Option<SyscallHandlerFn>; MAX_SYSCALL_NR] = [None; MAX_SYSCALL_NR];

/* ── MSR 读写 ── */

/// 写入 MSR 寄存器
#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    asm!("wrmsr", in("eax") low, in("edx") high, in("ecx") msr);
}

/* ── 初始化 ── */

/// 初始化 SYSCALL/SYSRET MSR 配置
///
/// 设置三个 MSR：
/// - STAR: syscall 使用 Ring0Code(0x08) 段，sysret 使用 Ring3Code(0x3B) 段
/// - LSTAR: syscall 入口地址指向 `syscall_entry`
/// - FMASK: 进入 Ring 0 时自动清除 IF（关中断）
///
/// 此函数必须在 GDT 初始化之后调用。
pub fn init() {
    unsafe {
        // STAR: bits[47:32] = syscall CS (Ring0Code=0x08)
        //        bits[63:48] = sysret CS (Ring3Code=0x3B)
        // sysret 时 SS = CS + 8 = 0x43 (Ring3Data)
        let star = (crate::arch::x86_64::gdt::selector::RING3_CODE as u64) << 48
                 | (crate::arch::x86_64::gdt::selector::RING0_CODE as u64) << 32;
        wrmsr(IA32_STAR, star);

        // LSTAR: Ring 0 入口点
        wrmsr(IA32_LSTAR, syscall_entry as u64);

        // FMASK: bit 9 = IF (Interrupt Flag)
        // 进入 Ring 0 时自动清 IF，防止 syscall 期间被中断打断
        wrmsr(IA32_FMASK, 0x200);
    }
}

/* ── syscall 入口（汇编 trampoline） ── */

/// syscall 入口 trampoline
///
/// CPU 自动完成：
/// - rcx = user RIP, r11 = user RFLAGS
/// - RSP 切换到 TSS.RSP0（Ring 0 栈）
/// - CS/SS 从 STAR MSR 加载
///
/// trampoline 保存所有寄存器到栈帧，调用 Rust syscall_handler，
/// 恢复寄存器后执行 sysret 返回 Ring 3。
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_entry() -> ! {
    naked_asm!(
        // ── 保存被调用者保存的寄存器 ──
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbp",
        "push rbx",

        // ── 保存参数（逆序压入，保持 args[0]=rdi 在低地址） ──
        "push r9",
        "push r8",
        "push r10",
        "push rdx",
        "push rsi",
        "push rdi",

        // ── 保存元数据 ──
        "push rax",     // syscall 编号
        "push rcx",     // user RIP
        "push r11",     // user RFLAGS

        // rdi = SyscallFrame 指针
        "mov rdi, rsp",
        "call syscall_handler",

        // 恢复 rcx, r11 供 sysret 使用
        "pop r11",
        "pop rcx",

        // 跳过 num + 6 args = 7×8 = 56 字节
        "add rsp, 56",

        // 恢复被调用者保存的寄存器
        "pop rbx",
        "pop rbp",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",

        // rax = 返回值（由 handler 设置）
        "sysret",
    )
}

/* ── 自定义处理函数管理 ── */

/// 注册自定义 syscall 处理函数
///
/// `nr` 为 syscall 编号，`handler` 为对应的处理函数。
/// 此函数注册的处理器在 `syscall::dispatch` 中可被查询和调用。
pub fn set_handler(nr: u64, handler: fn(frame: &SyscallFrame) -> u64) {
    if (nr as usize) < MAX_SYSCALL_NR {
        unsafe {
            SYSCALL_HANDLERS[nr as usize] = Some(handler);
        }
    }
}

/// 查询指定编号的自定义处理函数
pub fn get_handler(nr: u64) -> Option<SyscallHandlerFn> {
    if (nr as usize) < MAX_SYSCALL_NR {
        unsafe { SYSCALL_HANDLERS[nr as usize] }
    } else {
        None
    }
}
