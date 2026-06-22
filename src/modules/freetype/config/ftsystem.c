/*
 * ftsystem.c — FreeType 系统接口（Tungsten 内核裸机适配）
 * 基于 Bump Allocator 的内存管理；禁用文件 I/O（FT_New_Memory_Face 直接用内存加载字体）
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#include <ft2build.h>
#include FT_CONFIG_CONFIG_H
#include <freetype/ftsystem.h>
#include <freetype/fttypes.h>
#include <freetype/internal/ftdebug.h>

#include "ftalloc.h"

FT_CALLBACK_DEF(void *)
ft_alloc(FT_Memory memory, long size) {
    FT_UNUSED(memory);
    return ft_alloc_alloc((size_t)size);
}

FT_CALLBACK_DEF(void *)
ft_realloc(FT_Memory memory, long cur_size, long new_size, void *block) {
    FT_UNUSED(memory);
    if (!block)
        return ft_alloc_alloc((size_t)new_size);
    if (new_size <= 0) {
        ft_alloc_free(block);
        return NULL;
    }
    void *new_block = ft_alloc_alloc((size_t)new_size);
    if (new_block) {
        size_t copy = cur_size < new_size ? (size_t)cur_size : (size_t)new_size;
        ft_memcpy(new_block, block, copy);
    }
    ft_alloc_free(block);
    return new_block;
}

FT_CALLBACK_DEF(void)
ft_free(FT_Memory memory, void *block) {
    FT_UNUSED(memory);
    ft_alloc_free(block);
}

FT_BASE_DEF(FT_Memory)
FT_New_Memory(void) {
    ft_alloc_init();
    FT_Memory memory = (FT_Memory)ft_alloc_alloc(sizeof(*memory));
    if (memory) {
        memory->user    = NULL;
        memory->alloc   = ft_alloc;
        memory->realloc = ft_realloc;
        memory->free    = ft_free;
    }
    return memory;
}

FT_BASE_DEF(void)
FT_Done_Memory(FT_Memory memory) {
    ft_alloc_free(memory);
    ft_alloc_reset();
}
