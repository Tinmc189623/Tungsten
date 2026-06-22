// console/mod.rs — 帧缓冲控制台 (FreeType + OpenType 矢量字体)
//
// 规则 34-36 要求：禁止点阵字体，必须使用 FreeType + OpenType。
// 字体通过 include_bytes! 嵌入内核，FreeType 在运行时渲染字形。
// 帧缓冲由 Limine 引导加载器提供。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

mod framebuffer;

use framebuffer::Framebuffer;
use crate::bootinfo::BootInfo;
use crate::font_port;
use crate::serial;

/// 嵌入到内核中的等宽 OpenType 控制台字体
const CONSOLE_FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/console.otf");

/// 控制台字体像素尺寸 (宽 x 高)
const FONT_WIDTH: u32 = 8;
const FONT_HEIGHT: u32 = 16;

/// 全局帧缓冲控制台
pub struct Console {
    fb: Framebuffer,
    
    ft_lib: font_port::FT_Library,
    ft_face: font_port::FT_Face,
    cursor_x: usize,
    cursor_y: usize,
    fg_color: u32,
    bg_color: u32,
    cols: usize,
    rows: usize,
}

impl Console {
    /// 从 BootInfo 初始化控制台及 FreeType 字体引擎
    ///
    /// # Safety
    /// `boot_info` 必须指向有效的、由 Limine 填充的启动信息结构体
    pub unsafe fn new(boot_info: &BootInfo) -> Self {
        let fb = Framebuffer::new(boot_info);

        // 初始化 FreeType 库
        let mut ft_lib: font_port::FT_Library = core::ptr::null_mut();
        serial::write_str(b"  ft_init_freetype...\n");
        let err = font_port::FT_Init_FreeType(&mut ft_lib);
        serial::write_str(b"  ft_init_freetype done\n");
        if err != 0 || ft_lib.is_null() {
            serial::write_str(b"FT_Init_FreeType failed: err=");
            serial_hex(err as u64);
            serial::write_str(b" lib=");
            serial_hex(ft_lib as u64);
            serial::write_str(b"\n");
            fb.clear();
            loop { core::hint::spin_loop() }
        }

        // 从内存加载 OpenType 字体
        let mut ft_face: font_port::FT_Face = core::ptr::null_mut();
        serial::write_str(b"  ft_new_memory_face...\n");
        let err = font_port::FT_New_Memory_Face(
            ft_lib,
            CONSOLE_FONT_DATA.as_ptr(),
            CONSOLE_FONT_DATA.len() as i64,
            0,
            &mut ft_face,
        );
        serial::write_str(b"  ft_new_memory_face done, err=");
        serial_hex(err as u64);
        serial::write_str(b"\n");
        if err != 0 || ft_face.is_null() {
            fb.clear();
            loop { core::hint::spin_loop() }
        }

        // 设置字体像素尺寸
        font_port::FT_Set_Pixel_Sizes(ft_face, FONT_WIDTH, FONT_HEIGHT);

        // 计算控制台网格 (列数 x 行数)
        let fb_w = fb.width();
        let fb_h = fb.height();
        fb.clear();

        Console {
            cols: fb_w / FONT_WIDTH as usize,
            rows: fb_h / FONT_HEIGHT as usize,
            cursor_x: 0,
            cursor_y: 0,
            fg_color: 0xFFFFFF,
            bg_color: 0x000000,
            fb,
            ft_lib,
            ft_face,
        }
    }

    /// 输出单个字符到控制台 (支持 Unicode 全量字符)
    pub fn put_char(&mut self, ch: char) {
        match ch {
            '\n' => { self.cursor_x = 0; self.cursor_y += 1; }
            '\r' => { self.cursor_x = 0; }
            '\t' => { self.cursor_x = (self.cursor_x + 4) & !3; }
            c if (c as u32) < 0x20 => return,
            _ => {
                self.render_glyph(ch);
                self.cursor_x += 1;
            }
        }
        if self.cursor_x >= self.cols {
            self.cursor_x = 0;
            self.cursor_y += 1;
        }
        if self.cursor_y >= self.rows {
            self.scroll();
        }
    }

    /// 使用 FreeType 渲染单个字符字形并 blit 到帧缓冲
    fn render_glyph(&mut self, ch: char) {
        unsafe {
            // ch as u32 转换为 Unicode 码位，支持全量字符
            let err = font_port::FT_Load_Char(
                self.ft_face,
                ch as u64,
                font_port::FT_LOAD_RENDER | font_port::FT_LOAD_MONOCHROME,
            );
            if err != 0 { return; }

            // 获取字形槽位 (face->glyph)
            let slot = font_port::ft_helper_get_glyph_slot(self.ft_face);
            if slot.is_null() { return; }

            let bm = font_port::get_glyph_bitmap(slot);
            if bm.buffer.is_null() || bm.width == 0 || bm.rows == 0 { return; }

            // 计算帧缓冲上的目标位置
            let base_x = self.cursor_x * FONT_WIDTH as usize;
            let base_y = self.cursor_y * FONT_HEIGHT as usize;
            let off_x = bm.left as isize;
            let off_y = FONT_HEIGHT as isize - bm.top as isize;

            for row in 0..bm.rows as isize {
                for col in 0..bm.width as isize {
                    let byte_idx = (row * bm.pitch as isize + col / 8) as usize;
                    let bit = 0x80 >> (col & 7);
                    let pixel_on = (*bm.buffer.add(byte_idx) & bit) != 0;

                    let px = (base_x as isize + off_x + col) as usize;
                    let py = (base_y as isize + off_y + row) as usize;
                    let color = if pixel_on { self.fg_color } else { self.bg_color };
                    self.fb.put_pixel(px, py, color);
                }
            }
        }
    }

    /// 向上滚动一行 (内存拷贝 + 清除底部行)
    fn scroll(&mut self) {
        let row_bytes = self.fb.pitch() * FONT_HEIGHT as usize;
        let total_rows = self.rows;
        unsafe {
            let fb8 = self.fb.addr as *mut u8;
            core::ptr::copy(
                fb8.add(row_bytes),
                fb8,
                row_bytes * (total_rows - 1),
            );
        }
        // 清除底部行
        self.fb.fill_rect(
            0,
            (self.rows - 1) * FONT_HEIGHT as usize,
            self.fb.width(),
            FONT_HEIGHT as usize,
            self.bg_color,
        );
        self.cursor_y = self.rows - 1;
    }

    /// 输出字符串到控制台 (迭代 Unicode 字符而非字节)
    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.put_char(c);
        }
    }
}

/// 全局控制台实例
static mut CONSOLE: Option<Console> = None;

/// 初始化控制台
///
/// # Safety
/// 必须在 GDT 和 IDT 初始化之后、首次调用任何打印宏之前调用，
/// 且只应调用一次。
pub unsafe fn init(boot_info: &BootInfo) {
    CONSOLE = Some(Console::new(boot_info));
}

/// 输出字符串到控制台
pub fn write_str(s: &str) {
    unsafe {
        if let Some(ref mut con) = CONSOLE {
            con.write_str(s);
        }
    }
}

/// 输出格式化字符串到控制台

pub fn write_fmt(args: core::fmt::Arguments) {
    unsafe {
        if let Some(ref mut con) = CONSOLE {
            use core::fmt::Write;
            let _ = con.write_fmt(args);
        }
    }
}

/// 清屏并重置光标位置
pub unsafe fn clear() {
    if let Some(ref mut con) = CONSOLE {
        con.fb.clear();
        con.cursor_x = 0;
        con.cursor_y = 0;
    }
}

/// 获取帧缓冲宽度 (像素)
pub unsafe fn width() -> usize {
    unsafe { &*core::ptr::addr_of!(CONSOLE) }
        .as_ref().map(|c| c.fb.width()).unwrap_or(0)
}

/// 获取帧缓冲高度 (像素)
pub unsafe fn height() -> usize {
    unsafe { &*core::ptr::addr_of!(CONSOLE) }
        .as_ref().map(|c| c.fb.height()).unwrap_or(0)
}

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

/// 将 u64 格式化为十六进制并输出到串口 (仅调试用)
fn serial_hex(val: u64) {
    let mut buf = [0u8; 18];
    buf[0] = b'0'; buf[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> ((15 - i) * 4)) & 0xF) as u8;
        buf[i + 2] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
    }
    serial::write_str(&buf);
}

/// 格式化打印宏 (无换行)
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::write_fmt(format_args!($($arg)*));
    };
}

/// 格式化打印宏 (自动换行)
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n");
    };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*));
    };
}
