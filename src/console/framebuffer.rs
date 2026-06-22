// console/framebuffer.rs — 线性帧缓冲像素操作
//
// 像素读写委托给 Zig HAL (hal::framebuffer)。
// Limine 引导加载器提供线性帧缓冲，内核通过此模块访问。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::bootinfo::BootInfo;

/* ── Zig HAL 帧缓冲接口 ── */

#[link(name = "hal_tungsten", kind = "static")]
unsafe extern "C" {
    /// 初始化帧缓冲全局状态
    fn hal_fb_init(addr: u64, width: u32, height: u32, pitch: u32, bpp: u32);
    /// 绘制单个像素 (含边界检查)
    fn hal_fb_put_pixel(x: u32, y: u32, color: u32);
    /// 填充矩形区域
    fn hal_fb_fill_rect(x: u32, y: u32, w: u32, h: u32, color: u32);
    /// 清屏 (指定颜色)
    fn hal_fb_clear(color: u32);
}

/// 帧缓冲实例，封装 Limine 提供的线性帧缓冲参数
pub struct Framebuffer {
    pub(crate) addr: *mut u32,
    width: usize,
    height: usize,
    pitch: usize,
}

impl Framebuffer {
    /// 从 BootInfo 创建帧缓冲实例，并初始化 Zig HAL 全局状态
    ///
    /// # Safety
    /// `boot_info.fb_addr` 必须指向有效的帧缓冲内存区域
    pub unsafe fn new(boot_info: &BootInfo) -> Self {
        // 初始化 Zig HAL 帧缓冲全局状态
        hal_fb_init(
            boot_info.fb_addr,
            boot_info.fb_width as u32,
            boot_info.fb_height as u32,
            boot_info.fb_pitch as u32,
            boot_info.fb_bpp as u32,
        );
        Framebuffer {
            addr: boot_info.fb_addr as *mut u32,
            width: boot_info.fb_width as usize,
            height: boot_info.fb_height as usize,
            pitch: boot_info.fb_pitch as usize,
        }
    }

    /// 绘制单个像素点 (委托给 Zig HAL，含边界检查)
    pub fn put_pixel(&self, x: usize, y: usize, color: u32) {
        unsafe { hal_fb_put_pixel(x as u32, y as u32, color); }
    }

    /// 填充矩形区域为指定颜色 (委托给 Zig HAL)
    pub fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        unsafe { hal_fb_fill_rect(x as u32, y as u32, w as u32, h as u32, color); }
    }

    /// 清屏 (黑色)
    pub fn clear(&self) {
        unsafe { hal_fb_clear(0x000000); }
    }

    /// 帧缓冲宽度 (像素)
    pub fn width(&self) -> usize { self.width }

    /// 帧缓冲高度 (像素)
    pub fn height(&self) -> usize { self.height }

    /// 帧缓冲行距 (字节)
    pub fn pitch(&self) -> usize { self.pitch }
}
