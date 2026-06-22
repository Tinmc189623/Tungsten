/*
 * ftalloc.h — Tungsten 内核裸机 Bump Allocator（FreeType 专用）
 * 单一固定大小的 Arena，仅支持顺序分配（无释放），
 * 适合 FreeType 在启动初始化阶段的临时内存需求。
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#ifndef FTALLOC_H_
#define FTALLOC_H_

#include <stddef.h>

/* Arena 大小：256 KB（足以缓存常用字形的轮廓数据） */
#define FT_ALLOC_POOL_SIZE  (256 * 1024)

/* 初始化 bump allocator（在首次 FreeType 调用前调用） */
void ft_alloc_init(void);

/* 从 bump arena 分配一块对齐内存 */
void *ft_alloc_alloc(size_t size);

/* 释放（bump allocator：空操作） */
void ft_alloc_free(void *ptr);

/* 重置 arena（将当前位置重置回起始位置） */
void ft_alloc_reset(void);

#endif /* FTALLOC_H_ */
