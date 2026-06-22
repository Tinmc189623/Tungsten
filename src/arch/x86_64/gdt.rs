// gdt.rs — GDT + TSS 初始化（4-Ring 特权级架构）
// 为 x86_64 长模式构建完整的 4 层 GDT 表，
// 包含 Ring 0-3 代码段/数据段及 64 位 TSS 描述符
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::arch::asm;

/* ── TSS 结构 ── */

/// 64 位 TSS 结构（Intel Vol.3A §8.7 Figure 8-11）
///
/// 所有保留字段必须为零，TSS 总大小为 104 字节。
/// RSP0-RSP2 用于从 Ring 1/2/3 切换到 Ring 0 时的栈指针。
/// IST1-IST7 用于中断栈表（Double Fault、NMI 等使用独立栈）。
#[repr(C, packed)]
struct TaskStateSegment {
    _reserved1: u32,    // 0x00
    rsp: [u64; 3],      // 0x04 — RSP0, RSP1, RSP2
    _reserved2: u32,    // 0x1C — Intel 此处为 32 位保留
    ist: [u64; 7],      // 0x20 — IST1 ~ IST7
    _reserved3: u32,    // 0x58
    _reserved4: u32,    // 0x5C
    _reserved5: u16,    // 0x60
    iopb: u16,          // 0x62 — I/O 位图基址（为 0 时无位图）
}

/* ── GDT 结构 ── */

/// GDT 表（11 项：9 段描述符 + 1 个 TSS 描述符占 2 项）
///
/// 段选择子布局：
/// - [0]  Null           selector = 0x00
/// - [1]  Ring0Code      selector = 0x08 (DPL=0, kernel)
/// - [2]  Ring0Data      selector = 0x10
/// - [3]  Ring1Code      selector = 0x19 (DPL=1, drivers)
/// - [4]  Ring1Data      selector = 0x21
/// - [5]  Ring2Code      selector = 0x2A (DPL=2, I/O subsystems)
/// - [6]  Ring2Data      selector = 0x32
/// - [7]  Ring3Code      selector = 0x3B (DPL=3, userspace)
/// - [8]  Ring3Data      selector = 0x43
/// - [9]  TSS Low        selector = 0x48
/// - [10] TSS High
#[repr(C, align(8))]
struct Gdt {
    entries: [u64; 11],
}

/// GDTR 结构（10 字节：2 字节 Limit + 8 字节 Base）
#[repr(C, packed)]
struct Gdtr {
    limit: u16,
    base: u64,
}

/* ── 段选择子常量 ── */

/// 段选择子常量，供 LGDT/LTR/中断门/MSR 使用

pub mod selector {
    /// 空段选择子
    pub const NULL: u16        = 0x00;
    /// Ring 0 代码段（内核态）
    pub const RING0_CODE: u16  = 0x08;
    /// Ring 0 数据段（内核态）
    pub const RING0_DATA: u16  = 0x10;
    /// Ring 1 代码段（驱动层）
    pub const RING1_CODE: u16  = 0x19;
    /// Ring 1 数据段（驱动层）
    pub const RING1_DATA: u16  = 0x21;
    /// Ring 2 代码段（I/O 子系统、文件系统）
    pub const RING2_CODE: u16  = 0x2A;
    /// Ring 2 数据段（I/O 子系统、文件系统）
    pub const RING2_DATA: u16  = 0x32;
    /// Ring 3 代码段（用户空间）
    pub const RING3_CODE: u16  = 0x3B;
    /// Ring 3 数据段（用户空间）
    pub const RING3_DATA: u16  = 0x43;
    /// TSS 段选择子
    pub const TSS: u16         = 0x48;
}

/* ── 段描述符构造函数 ── */

/// 创建 x86_64 长模式代码段描述符
///
/// 位布局：Type=0xA(code,exec/read,accessed), S=1, DPL=ring, P=1, L=1(long mode), G=1(4K)
const fn code_descriptor(dpl: u8) -> u64 {
    let mut desc: u64 = 0;
    desc |= 0x0A << 40;          // Type: code, execute/read, accessed
    desc |= 1 << 44;             // S: 1 = code/data descriptor
    desc |= (dpl as u64) << 45;  // DPL
    desc |= 1 << 47;             // P: present
    desc |= 1 << 53;             // L: long mode (64-bit code)
    desc |= 1 << 55;             // G: granularity (4K page)
    desc
}

/// 创建 x86_64 数据段描述符
///
/// 位布局：Type=0x2(data,r/w,accessed), S=1, DPL=ring, P=1, G=1(4K)
const fn data_descriptor(dpl: u8) -> u64 {
    let mut desc: u64 = 0;
    desc |= 0x02 << 40;          // Type: data, read/write, accessed
    desc |= 1 << 44;             // S: 1 = code/data descriptor
    desc |= (dpl as u64) << 45;  // DPL
    desc |= 1 << 47;             // P: present
    desc |= 1 << 55;             // G: granularity (4K page)
    desc
}

/// 创建 64 位 TSS 描述符低 64 位
///
/// 位布局：Type=0x9(available 64-bit TSS), S=0, P=1, 其余为 base/limit 编码
const fn tss_descriptor_low(base: u64, limit: u32) -> u64 {
    let mut desc: u64 = 0;
    desc |= (limit as u64 & 0xFFFF) << 0;        // Limit[15:0]
    desc |= (base & 0xFFFFFF) << 16;              // Base[23:0]
    desc |= 0x09 << 40;                           // Type: available 64-bit TSS
    desc |= 1 << 47;                              // P: present
    desc |= ((limit as u64 >> 16) & 0x0F) << 48;  // Limit[19:16]
    desc |= ((base >> 24) & 0xFF) << 56;          // Base[31:24]
    desc
}

/// 创建 64 位 TSS 描述符高 64 位（Base[63:32]）
const fn tss_descriptor_high(base: u64) -> u64 {
    base >> 32
}

/* ── 全局静态实例 ── */

/// 全局 TSS 实例（编译期预填充默认栈地址）
///
/// RSP0 = Ring 0 栈（内核态），RSP1 = Ring 1 栈（驱动层），RSP2 = Ring 2 栈（I/O 层）
/// IST1 = Double Fault 独立栈，IST2 = NMI 独立栈
static mut TSS: TaskStateSegment = TaskStateSegment {
    _reserved1: 0,
    rsp: [
        0x9_F000,  // RSP0: Ring 0 栈
        0x9_E000,  // RSP1: Ring 1 栈
        0x9_D000,  // RSP2: Ring 2 栈
    ],
    _reserved2: 0,
    ist: [
        0x9_EC00,  // IST1: Double Fault
        0x9_E800,  // IST2: NMI
        0, 0, 0, 0, 0,
    ],
    _reserved3: 0,
    _reserved4: 0,
    _reserved5: 0,
    iopb: 0,
};

/// 全局 GDT 实例（编译期预填充所有静态段描述符，TSS 项在 init() 中运行时填充）
static mut GDT: Gdt = Gdt {
    entries: [
        0,                     // [0]  Null 段
        code_descriptor(0),    // [1]  Ring0Code    selector=0x08
        data_descriptor(0),    // [2]  Ring0Data    selector=0x10
        code_descriptor(1),    // [3]  Ring1Code    selector=0x19
        data_descriptor(1),    // [4]  Ring1Data    selector=0x21
        code_descriptor(2),    // [5]  Ring2Code    selector=0x2A
        data_descriptor(2),    // [6]  Ring2Data    selector=0x32
        code_descriptor(3),    // [7]  Ring3Code    selector=0x3B
        data_descriptor(3),    // [8]  Ring3Data    selector=0x43
        0,                     // [9]  TssLow       (runtime)
        0,                     // [10] TssHigh      (runtime)
    ],
};

/* ── 公开 API ── */

/// 初始化 GDT 和 TSS
///
/// 运行时完成两步填充:
/// 1. TSS 描述符写入 GDT（需要 TSS 的运行时地址，编译期未知）
/// 2. 加载 GDTR 寄存器和任务寄存器 (LTR)
///
/// 此函数必须在启动早期调用，在 IDT 初始化之前。
pub fn init() {
    unsafe {
        // TSS 描述符依赖运行时基址
        let tss_base = core::ptr::addr_of!(TSS) as u64;
        let tss_limit = core::mem::size_of::<TaskStateSegment>() as u32 - 1;

        // 通过原始指针写入 GDT 条目，避免创建对 static mut 的引用
        let gdt_ptr: *mut Gdt = core::ptr::addr_of_mut!(GDT);
        (*gdt_ptr).entries[9] = tss_descriptor_low(tss_base, tss_limit);
        (*gdt_ptr).entries[10] = tss_descriptor_high(tss_base);

        // 构建并加载 GDTR
        let gdtr = Gdtr {
            limit: (core::mem::size_of::<Gdt>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };
        asm!("lgdt [{0}]", in(reg) &gdtr, options(readonly, nostack, preserves_flags));

        // 加载任务寄存器（TSS 选择子 0x48）
        asm!("ltr {0:x}", in(reg) selector::TSS, options(nostack, preserves_flags));
    }
}

/// 设置 TSS 中指定 Ring 的栈指针
///
/// `ring` 取值范围 0-2，分别对应 RSP0（Ring 0 栈）、RSP1（Ring 1 栈）、RSP2（Ring 2 栈）。
/// 当 CPU 从外层特权级通过中断/调用门切换到内层时，硬件自动从此处加载栈指针。
///
/// # Safety
/// `stack_top` 必须指向有效且已映射的栈顶地址。
pub fn set_tss_stack(ring: u8, stack_top: u64) {
    unsafe {
        let tss = core::ptr::addr_of_mut!(TSS);
        match ring {
            0 => (*tss).rsp[0] = stack_top,
            1 => (*tss).rsp[1] = stack_top,
            2 => (*tss).rsp[2] = stack_top,
            _ => {}
        }
    }
}

/// 设置 TSS 中的 IST（Interrupt Stack Table）条目
///
/// `index` 取值 1-7，对应 IST1-IST7。
/// 中断门可指定 IST 索引以切换到独立栈，用于 Double Fault/NMI 等场景。
///
/// # Safety
/// `stack_top` 必须指向有效且已映射的栈顶地址。
pub fn set_ist(index: u8, stack_top: u64) {
    unsafe {
        let tss = core::ptr::addr_of_mut!(TSS);
        if index >= 1 && index <= 7 {
            (*tss).ist[(index - 1) as usize] = stack_top;
        }
    }
}
