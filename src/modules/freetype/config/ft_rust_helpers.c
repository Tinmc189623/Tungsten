/*
 * ft_rust_helpers.c — 供 Rust FFI 调用的 FreeType 辅助函数
 * 提取渲染后字形位图数据，避免 Rust 侧直接匹配 C 结构体布局
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#include <ft2build.h>
#include FT_FREETYPE_H

/* ── 位图数据访问 ── */

unsigned int ft_helper_bitmap_width(FT_GlyphSlot slot) {
    return slot->bitmap.width;
}

unsigned int ft_helper_bitmap_rows(FT_GlyphSlot slot) {
    return slot->bitmap.rows;
}

int ft_helper_bitmap_pitch(FT_GlyphSlot slot) {
    return slot->bitmap.pitch;
}

unsigned char *ft_helper_bitmap_buffer(FT_GlyphSlot slot) {
    return slot->bitmap.buffer;
}

int ft_helper_bitmap_left(FT_GlyphSlot slot) {
    return slot->bitmap_left;
}

int ft_helper_bitmap_top(FT_GlyphSlot slot) {
    return slot->bitmap_top;
}

long ft_helper_advance_x(FT_GlyphSlot slot) {
    return slot->advance.x;
}

/* ── 字形槽位访问 ── */

FT_GlyphSlot ft_helper_get_glyph_slot(FT_Face face) {
    return face->glyph;
}
