// arch/x86_64/mod.rs — x86_64 架构模块根
// 导出 GDT、IDT、syscall、ACPI、APIC 子模块及 SSE 初始化
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod gdt;
pub mod idt;
pub mod acpi;
pub mod apic;
pub mod syscall;

/// 启用 SSE/SSE2 指令集支持
///
/// x86_64 长模式下 SSE/SSE2 是 baseline 要求，但内核必须在
/// CR0/CR4 中显式启用相关控制位。Rust 编译器、FreeType、memcpy
/// 等均可能生成 SSE 指令，缺失此初始化会导致 #UD (Invalid Opcode) 异常。
///
/// 必须在任何可能使用浮点/SIMD 指令的代码之前调用。
pub fn enable_sse() {
    unsafe {
        // CR0: 清 EM (Emulation)，置 MP (Monitor Coprocessor)
        let mut cr0: u64;
        core::arch::asm!("mov {0}, cr0", out(reg) cr0);
        cr0 &= !(1 << 2);  // 清 CR0.EM: 禁止 x87 FPU 指令模拟
        cr0 |= 1 << 1;     // 置 CR0.MP: 与 TS 配合用于 lazy FPU 上下文切换
        core::arch::asm!("mov cr0, {0}", in(reg) cr0);

        // CR4: 置 OSFXSR + OSXMMEXCPT
        let mut cr4: u64;
        core::arch::asm!("mov {0}, cr4", out(reg) cr4);
        cr4 |= (1 << 9) | (1 << 10);  // OSFXSR = SSE 状态保存, OSXMMEXCPT = SIMD 异常处理
        core::arch::asm!("mov cr4, {0}", in(reg) cr4);
    }
}
