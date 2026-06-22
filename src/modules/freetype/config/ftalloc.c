/*
 * ftalloc.c — Tungsten 内核 Bump Allocator for FreeType
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#include "ftalloc.h"
#include <stdint.h>

/* 静态 Arena 缓冲区（BSS 段，不占用二进制体积） */
static char alloc_pool[FT_ALLOC_POOL_SIZE];
static size_t alloc_offset = 0;

void ft_alloc_init(void) {
    alloc_offset = 0;
}

void *ft_alloc_alloc(size_t size) {
    /* 8 字节对齐 */
    size_t align = 8;
    size_t mask = align - 1;
    size_t aligned = (alloc_offset + mask) & ~mask;

    if (aligned + size > FT_ALLOC_POOL_SIZE)
        return (void *)0;  /* OOM */

    alloc_offset = aligned + size;
    return (void *)&alloc_pool[aligned];
}

void ft_alloc_free(void *ptr) {
    (void)ptr;
    /* Bump allocator 不做单次释放 */
}

void ft_alloc_reset(void) {
    alloc_offset = 0;
}
