// font_port.rs — 内核字体端口层（自研 Rust 绑定）
// 通过静态链接调用 src/modules/freetype 编译产物，内核树内不含第三方 C 源码
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(non_camel_case_types, dead_code)]

/// 字体引擎错误码（成功为 0）
pub type FT_Error = i32;

/* ── 不透明句柄 ── */

/// FT_LibraryRec_* — 不透明指针
pub enum FT_LibraryRec_ {}
pub type FT_Library = *mut FT_LibraryRec_;

/// FT_FaceRec_* — 不透明指针
pub enum FT_FaceRec_ {}
pub type FT_Face = *mut FT_FaceRec_;

/// FT_GlyphSlotRec_* — 不透明指针（从 FT_Face.glyph 获取）
pub enum FT_GlyphSlotRec_ {}
pub type FT_GlyphSlot = *mut FT_GlyphSlotRec_;

/* ── 外部字体模块 C API（由 build.rs 链接 libfreetype_tungsten.a） ── */

#[link(name = "freetype_tungsten", kind = "static")]
unsafe extern "C" {
    /// 初始化字体库
    pub fn FT_Init_FreeType(alibrary: *mut FT_Library) -> FT_Error;

    /// 从内存缓冲区打开字体
    pub fn FT_New_Memory_Face(
        library: FT_Library,
        file_base: *const u8,
        file_size: i64,
        face_index: i64,
        aface: *mut FT_Face,
    ) -> FT_Error;

    /// 设置字符像素尺寸
    pub fn FT_Set_Pixel_Sizes(
        face: FT_Face,
        pixel_width: u32,
        pixel_height: u32,
    ) -> FT_Error;

    /// 加载并渲染字形
    pub fn FT_Load_Char(
        face: FT_Face,
        char_code: u64,
        load_flags: i32,
    ) -> FT_Error;
}

/* ── 位图辅助（C 桩，位于 modules/freetype/config） ── */

unsafe extern "C" {
    /// 获取字形槽位指针（face->glyph）
    pub fn ft_helper_get_glyph_slot(face: FT_Face) -> FT_GlyphSlot;

    /// 位图宽度（像素）
    pub fn ft_helper_bitmap_width(slot: FT_GlyphSlot) -> u32;

    /// 位图行数（像素）
    pub fn ft_helper_bitmap_rows(slot: FT_GlyphSlot) -> u32;

    /// 每行字节数
    pub fn ft_helper_bitmap_pitch(slot: FT_GlyphSlot) -> i32;

    /// 位图像素数据缓冲区
    pub fn ft_helper_bitmap_buffer(slot: FT_GlyphSlot) -> *mut u8;

    /// 位图左偏移
    pub fn ft_helper_bitmap_left(slot: FT_GlyphSlot) -> i32;

    /// 位图上偏移
    pub fn ft_helper_bitmap_top(slot: FT_GlyphSlot) -> i32;

    /// 水平步进（26.6 定点数）
    pub fn ft_helper_advance_x(slot: FT_GlyphSlot) -> i64;
}

/* ── 加载标志 ── */

/// FT_LOAD_RENDER：加载字形并渲染到位图
pub const FT_LOAD_RENDER: i32 = 4;

/// FT_LOAD_NO_BITMAP：不加载嵌入式位图（使用轮廓）
pub const FT_LOAD_NO_BITMAP: i32 = 0x8;

/// FT_LOAD_MONOCHROME：使用 1 位单色渲染
pub const FT_LOAD_MONOCHROME: i32 = 0x1000;

/* ── 字形位图封装 ── */

/// 渲染后的字形位图信息
pub struct GlyphBitmap {
    pub buffer: *mut u8,
    pub width: u32,
    pub rows: u32,
    pub pitch: i32,
    pub left: i32,
    pub top: i32,
    pub advance_x: i64,
}

/// 从已渲染的字形槽位提取位图信息
pub fn get_glyph_bitmap(slot: FT_GlyphSlot) -> GlyphBitmap {
    unsafe {
        GlyphBitmap {
            buffer: ft_helper_bitmap_buffer(slot),
            width: ft_helper_bitmap_width(slot),
            rows: ft_helper_bitmap_rows(slot),
            pitch: ft_helper_bitmap_pitch(slot),
            left: ft_helper_bitmap_left(slot),
            top: ft_helper_bitmap_top(slot),
            advance_x: ft_helper_advance_x(slot),
        }
    }
}
