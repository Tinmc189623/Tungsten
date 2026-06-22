// idt.rs — 中断描述符表 + 异常处理（256 向量完整 IDT）
// 0~31 CPU 异常，32~47 IRQ（APIC 映射），48~255 扩展中断/软件中断
// 0x80 保留为软件中断门（legacy syscall 兼容）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::arch::asm;
use core::arch::naked_asm;
use crate::arch::x86_64::gdt::selector;
use crate::arch::apic;

/* ── 数据结构 ── */

/// 中断门描述符（x86_64 格式，16 字节）
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,   // Handler RIP[15:0]
    selector: u16,     // 代码段选择子
    ist: u8,           // Interrupt Stack Table（仅低 3 位有效）
    flags: u8,         // 门类型、DPL、Present
    offset_mid: u16,   // Handler RIP[31:16]
    offset_high: u32,  // Handler RIP[63:32]
    _reserved: u32,
}

/// IDTR 结构（10 字节）
#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

/// IDT 表（256 项）
#[repr(C, align(8))]
struct Idt {
    entries: [IdtEntry; 256],
}

/// 异常/中断栈帧
///
/// exception_common / irq_common 保存全部通用寄存器后，
/// rdi 指向此结构体开头。
#[repr(C)]

pub struct ExceptionStack {
    /// exception_common 压入的 15 个通用寄存器（rax 最后压入 = offset 0）
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64,
    pub r8:  u64, pub r9:  u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    /// 入口 trampoline 压入的向量号
    pub vector: u64,
    /// 错误码：有错误码异常由 CPU 压入，无错误码异常由入口压入 0
    pub error_code: u64,
    /// CPU 自动压入的栈帧
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/* ── CPU 异常名称 ── */

/// CPU 异常名称（向量 0~31）

static EXCEPTION_NAMES: [&str; 32] = [
    "Divide Error",                    // 0  #DE
    "Debug",                           // 1  #DB
    "Non-Maskable Interrupt",          // 2
    "Breakpoint",                      // 3  #BP
    "Overflow",                        // 4  #OF
    "Bound Range Exceeded",            // 5  #BR
    "Invalid Opcode",                  // 6  #UD
    "Device Not Available",            // 7  #NM
    "Double Fault",                    // 8  #DF
    "Coprocessor Segment Overrun",     // 9  (reserved)
    "Invalid TSS",                     // 10 #TS
    "Segment Not Present",             // 11 #NP
    "Stack-Segment Fault",             // 12 #SS
    "General Protection Fault",        // 13 #GP
    "Page Fault",                      // 14 #PF
    "x86 Reserved",                    // 15
    "x87 FPU Floating-Point Error",    // 16 #MF
    "Alignment Check",                 // 17 #AC
    "Machine Check",                   // 18 #MC
    "SIMD Floating-Point Exception",   // 19 #XM
    "Virtualization Exception",        // 20 #VE
    "Control Protection Exception",    // 21 #CP
    "Reserved",                        // 22
    "Reserved",                        // 23
    "Reserved",                        // 24
    "Reserved",                        // 25
    "Reserved",                        // 26
    "Reserved",                        // 27
    "Hypervisor Injection Exception",  // 28 #HV
    "VMM Communication Exception",     // 29 #VC
    "Security Exception",              // 30 #SX
    "Reserved",                        // 31
];

/* ── 自定义 IRQ 处理表 ── */

/// 最大可注册的自定义 IRQ 处理函数数量
const MAX_IRQ_HANDLERS: usize = 256;

/// 自定义 IRQ 处理函数类型：接收向量号和错误码
type IrqHandlerFn = fn(vector: u8, error_code: u64);

/// 自定义处理函数注册表（向量号 → 函数指针）
static mut IRQ_HANDLERS: [Option<IrqHandlerFn>; MAX_IRQ_HANDLERS] = [None; MAX_IRQ_HANDLERS];

/* ── 静态 IDT ── */

/// 静态 IDT 实例（编译期零初始化）
static mut IDT: Idt = Idt {
    entries: [IdtEntry {
        offset_low: 0,
        selector: 0,
        ist: 0,
        flags: 0,
        offset_mid: 0,
        offset_high: 0,
        _reserved: 0,
    }; 256],
};

/// 写入 IDT 表中某一项
///
/// # Safety
/// vector 必须 < 256；handler 必须指向有效可执行代码；此函数修改全局 mutable static。
unsafe fn set_gate(vector: u8, handler: u64, seg_sel: u16, flags: u8, ist: u8) {
    unsafe {
        IDT.entries[vector as usize] = IdtEntry {
            offset_low: handler as u16,
            selector: seg_sel,
            ist: ist & 0x7,
            flags,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        };
    }
}

/* ── 异常入口点（汇编 trampoline） ── */

/// 无错误码异常：压入伪错误码 0，再压入向量号，跳转公共入口。
macro_rules! handler_no_error {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name() -> ! {
            naked_asm!(
                "push 0",
                "push {vec}",
                "jmp  exception_common",
                vec = const $vec,
            )
        }
    };
}

/// 有错误码异常：CPU 已压入错误码，仅压入向量号后跳转公共入口。
macro_rules! handler_with_error {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name() -> ! {
            naked_asm!(
                "push {vec}",
                "jmp  exception_common",
                vec = const $vec,
            )
        }
    };
}

// 向量 0–7：无错误码
handler_no_error!(exception_handler_0, 0);
handler_no_error!(exception_handler_1, 1);
handler_no_error!(exception_handler_2, 2);
handler_no_error!(exception_handler_3, 3);
handler_no_error!(exception_handler_4, 4);
handler_no_error!(exception_handler_5, 5);
handler_no_error!(exception_handler_6, 6);
handler_no_error!(exception_handler_7, 7);

// 向量 8：Double Fault — 有错误码，使用 TSS IST1
handler_with_error!(exception_handler_8, 8);

// 向量 9：保留（无错误码）
handler_no_error!(exception_handler_9, 9);

// 向量 10–13：有错误码
handler_with_error!(exception_handler_10, 10);
handler_with_error!(exception_handler_11, 11);
handler_with_error!(exception_handler_12, 12);
handler_with_error!(exception_handler_13, 13);

// 向量 14：Page Fault — 有错误码，独立命名以便缺页处理
/// Page Fault 入口，向量 14，CPU 自动压入错误码
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn page_fault_handler() -> ! {
    naked_asm!(
        "push 14",
        "jmp  exception_common",
    )
}

// 向量 15：保留（无错误码）
handler_no_error!(exception_handler_15, 15);

// 向量 16：x87 FPU Error（无错误码）
handler_no_error!(exception_handler_16, 16);

// 向量 17：Alignment Check — 有错误码
handler_with_error!(exception_handler_17, 17);

// 向量 18–19：无错误码
handler_no_error!(exception_handler_18, 18);
handler_no_error!(exception_handler_19, 19);

// 向量 20–29：保留/新型异常（无错误码）
handler_no_error!(exception_handler_20, 20);
handler_no_error!(exception_handler_21, 21);
handler_no_error!(exception_handler_22, 22);
handler_no_error!(exception_handler_23, 23);
handler_no_error!(exception_handler_24, 24);
handler_no_error!(exception_handler_25, 25);
handler_no_error!(exception_handler_26, 26);
handler_no_error!(exception_handler_27, 27);
handler_no_error!(exception_handler_28, 28);
handler_no_error!(exception_handler_29, 29);

// 向量 30：Security Exception — 有错误码
handler_with_error!(exception_handler_30, 30);

// 向量 31：保留（无错误码）
handler_no_error!(exception_handler_31, 31);

/* ── IRQ 入口（向量 32～255，外部中断） ── */

// IRQ 桩表：每个桩 16 字节（NOP 填充对齐），push 0 + push vec + jmp
core::arch::global_asm!(
    ".pushsection .text.irq_stubs, \"ax\"",
    ".align 16",
    "irq_stubs_start:",
    ".set  vec, 32",
    ".rept 224",
    "    push 0",
    "    push vec",
    "    jmp  irq_common",
    "    .align 16, 0x90",
    "    .set  vec, vec + 1",
    ".endr",
    ".popsection",
);

unsafe extern "C" {
    static irq_stubs_start: u8;
}

/// 获取 IRQ 向量对应的桩入口地址
fn irq_stub_addr(vec: u8) -> u64 {
    let base = core::ptr::addr_of!(irq_stubs_start) as u64;
    base + ((vec as u64 - 32) * 16)
}

/// IRQ 公共入口（汇编）：保存寄存器 → 调用 Rust irq_handler → EOI → iretq
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn irq_common() -> ! {
    core::arch::naked_asm!(
        "push r15", "push r14", "push r13", "push r12",
        "push r11", "push r10", "push r9",  "push r8",
        "push rbp", "push rdi", "push rsi",
        "push rdx", "push rcx", "push rbx", "push rax",
        "mov  rdi, rsp",
        "call irq_dispatch",
        "add  rsp, {stack_skip}",
        "iretq",
        stack_skip = const 136,
    )
}

/// Rust 层 IRQ 分派逻辑
///
/// 优先调用用户注册的自定义处理函数，然后处理内置中断（定时器、PS/2 键盘）。
/// 最后发送 EOI 通知 APIC 中断已结束。
#[unsafe(no_mangle)]
extern "C" fn irq_dispatch(stack: &ExceptionStack) {
    let vec = stack.vector as u8;

    // 检查自定义处理函数
    unsafe {
        if let Some(handler) = IRQ_HANDLERS[vec as usize] {
            handler(vec, stack.error_code);
        }
    }

    if vec >= 32 {
        // 内置：APIC 定时器中断 → 调度器 tick
        if vec == apic::TIMER_VECTOR {
            crate::sched::tick();
        }
        // 内置：PS/2 键盘中断（IRQ 1 = 向量 IRQ_BASE + 1 = 33）
        if vec == (apic::IRQ_BASE + 1) {
            crate::devices::input::ps2::irq_handler();
        }
        apic::eoi();
    }
}

/// 公共异常处理入口（汇编）
///
/// 1. 保存全部 15 个通用寄存器到栈
/// 2. rdi = rsp，交给 Rust exception_handler
/// 3. 返回后跳过已保存寄存器 + vector + error_code，iretq 恢复上下文
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_common() -> ! {
    naked_asm!(
        "push r15", "push r14", "push r13", "push r12",
        "push r11", "push r10", "push r9",  "push r8",
        "push rbp", "push rdi", "push rsi",
        "push rdx", "push rcx", "push rbx", "push rax",
        "mov  rdi, rsp",
        "call exception_handler",
        "add  rsp, {stack_skip}",
        "iretq",
        stack_skip = const 136,
    )
}

/// Rust 层异常处理函数
///
/// 通过串口输出异常信息后进入死循环（Kernel Panic）。
#[unsafe(no_mangle)]
extern "C" fn exception_handler(stack: &ExceptionStack) {
    let name = if (stack.vector as usize) < 32 {
        EXCEPTION_NAMES[stack.vector as usize]
    } else {
        "Unknown Interrupt"
    };
    let error = stack.error_code;
    let rip = stack.rip;

    crate::serial::write_str(b"\n=== KERNEL PANIC ===\n");
    crate::serial::write_str(b"Exception: ");
    crate::serial::write_str(name.as_bytes());
    crate::serial::write_str(b"\nVector: ");
    crate::serial::write_str(&hex_str(stack.vector));
    crate::serial::write_str(b"\nError:  ");
    crate::serial::write_str(&hex_str(error));
    crate::serial::write_str(b"\nRIP:    ");
    crate::serial::write_str(&hex_str(rip));
    crate::serial::write_str(b"\n");

    // Page Fault 时输出 CR2（缺页地址）
    if stack.vector == 14 {
        let cr2: u64;
        unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }
        crate::serial::write_str(b"CR2:    ");
        crate::serial::write_str(&hex_str(cr2));
        crate::serial::write_str(b"\n");
    }

    loop {
        core::hint::spin_loop()
    }
}

/// 将 u64 格式化为十六进制 ASCII 字符串（"0x0000000000000000"），无堆分配
fn hex_str(val: u64) -> [u8; 18] {
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> ((15 - i) * 4)) & 0xF) as u8;
        buf[i + 2] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
    }
    buf
}

/* ── 公开 API ── */

/// 初始化 IDT 并加载 IDTR
///
/// 注册所有 256 个中断/异常门：
/// - 向量 0-31：CPU 异常（#DF 使用 IST1，#PF 使用独立入口）
/// - 向量 32-255：外部中断（IRQ 桩表）
/// - 向量 0x80：DPL=3 陷阱门（legacy syscall 软件中断兼容）
///
/// 所有异常门标记为中断门（flags = 0x8E: Present, DPL=0, 64-bit Interrupt Gate）。
pub fn init() {
    unsafe {
        // ── CPU 异常向量 0-31 ──
        set_gate(0,  exception_handler_0  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(1,  exception_handler_1  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(2,  exception_handler_2  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(3,  exception_handler_3  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(4,  exception_handler_4  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(5,  exception_handler_5  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(6,  exception_handler_6  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(7,  exception_handler_7  as u64, selector::RING0_CODE, 0x8E, 0);
        // #DF 使用 IST 1
        set_gate(8,  exception_handler_8  as u64, selector::RING0_CODE, 0x8E, 1);
        set_gate(9,  exception_handler_9  as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(10, exception_handler_10 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(11, exception_handler_11 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(12, exception_handler_12 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(13, exception_handler_13 as u64, selector::RING0_CODE, 0x8E, 0);
        // #PF 使用独立入口
        set_gate(14, page_fault_handler   as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(15, exception_handler_15 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(16, exception_handler_16 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(17, exception_handler_17 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(18, exception_handler_18 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(19, exception_handler_19 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(20, exception_handler_20 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(21, exception_handler_21 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(22, exception_handler_22 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(23, exception_handler_23 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(24, exception_handler_24 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(25, exception_handler_25 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(26, exception_handler_26 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(27, exception_handler_27 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(28, exception_handler_28 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(29, exception_handler_29 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(30, exception_handler_30 as u64, selector::RING0_CODE, 0x8E, 0);
        set_gate(31, exception_handler_31 as u64, selector::RING0_CODE, 0x8E, 0);

        // ── IRQ 向量 32-255 ──
        for vec in 32..=255u8 {
            set_gate(vec, irq_stub_addr(vec), selector::RING0_CODE, 0x8E, 0);
        }

        // ── 向量 0x80：legacy syscall 软件中断门（DPL=3，用户态可触发） ──
        // 使用陷阱门标志 0xEE: Present, DPL=3, 64-bit Trap Gate
        // 暂指向通用 IRQ 桩（syscall 模块初始化后可替换）
        set_gate(0x80, irq_stub_addr(0x80), selector::RING0_CODE, 0xEE, 0);

        // ── 加载 IDTR ──
        let idtr = Idtr {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{0}]", in(reg) &idtr, options(readonly, nostack, preserves_flags));
    }
}

/// 注册自定义 IRQ 处理函数
///
/// 当指定向量的中断触发时，`irq_dispatch` 会优先调用此注册的函数，
/// 然后再执行内置处理逻辑（定时器 tick、键盘等）。
///
/// `irq` 为向量号（32-255 为外部中断）。
/// `handler` 为处理函数，参数为 (vector, error_code)。
pub fn register_handler(irq: u8, handler: fn(vector: u8, error_code: u64)) {
    unsafe {
        IRQ_HANDLERS[irq as usize] = Some(handler);
    }
}

/// 取消注册指定向量的自定义处理函数
pub fn unregister_handler(irq: u8) {
    unsafe {
        IRQ_HANDLERS[irq as usize] = None;
    }
}
