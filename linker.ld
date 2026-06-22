/* linker.ld — Tungsten 内核链接脚本 */
/* Copyright (C) 2026 Nexsteaduser. All rights reserved. */
/* SPDX-License-Identifier: GPL-3.0-or-later */

ENTRY(_start)

PHDRS {
    text    PT_LOAD FLAGS(5);  /* R+X */
    rodata  PT_LOAD FLAGS(4);  /* R   */
    data    PT_LOAD FLAGS(6);  /* R+W */
}

SECTIONS {
    . = 0xFFFFFFFF80000000 + 0x100000;

    /* Limine 协议请求段 */
    .limine_reqs : ALIGN(8) {
        KEEP(*(.limine_reqs_start));
        KEEP(*(.limine_reqs_section));
        KEEP(*(.limine_reqs));
        KEEP(*(.limine_reqs_end));
    } :text

    /* 代码段 */
    .text : ALIGN(4096) {
        *(.text._start);
        *(.text*);
    } :text

    /* 只读数据段 */
    .rodata : ALIGN(4096) {
        *(.rodata*);
    } :rodata

    /* 只读重定位数据 */
    .data.rel.ro : ALIGN(4096) {
        *(.data.rel.ro*);
    } :data

    /* 全局偏移表 */
    .got : ALIGN(4096) {
        *(.got*);
    } :data

    /* 可写数据段 */
    .data : ALIGN(4096) {
        *(.data*);
    } :data

    /* 未初始化数据段 */
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss*);
        *(COMMON);
        __bss_end = .;
    } :data

    /* 内核符号边界 */
    __kernel_start = 0xFFFFFFFF80100000;
    __kernel_end = .;

    /* 丢弃不需要的段 */
    /DISCARD/ : {
        *(.eh_frame*);
        *(.comment*);
        *(.note*);
    }
}
