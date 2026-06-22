// framebuffer.zig — 线性帧缓冲像素操作 (32bpp)
// 管理 Limine 提供的线性帧缓冲，向 Rust 内核提供绘图原语
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

/// 帧缓冲信息结构，与 Rust 侧通过 C ABI 传递
pub const FbInfo = extern struct {
    /// 帧缓冲物理地址 (线性映射后的虚拟地址)
    addr: u64,
    /// 水平分辨率 (像素数)
    width: u32,
    /// 垂直分辨率 (像素数)
    height: u32,
    /// 每行字节数 (可能含对齐填充)
    pitch: u32,
    /// 每像素位数 (通常为 32)
    bpp: u32,
};

// 模块内部状态，由 hal_fb_init 填充
var fb_addr: [*]volatile u32 = undefined;
var fb_width: u32 = 0;
var fb_height: u32 = 0;
var fb_pitch: u32 = 0;
var fb_bpp: u32 = 0;
var fb_initialized: bool = false;

/// 初始化帧缓冲，记录地址和几何参数
/// addr 为 Limine 传递的帧缓冲基地址 (已映射到虚拟地址空间)
export fn hal_fb_init(addr: u64, width: u32, height: u32, pitch: u32, bpp: u32) void {
    fb_addr = @ptrFromInt(addr);
    fb_width = width;
    fb_height = height;
    fb_pitch = pitch;
    fb_bpp = bpp;
    fb_initialized = true;
}

/// 在指定坐标绘制单个像素点
/// 坐标超出屏幕范围时静默忽略，color 为 0x00RRGGBB 格式
export fn hal_fb_put_pixel(x: u32, y: u32, color: u32) void {
    if (!fb_initialized) return;
    if (x >= fb_width or y >= fb_height) return;
    const row_words = fb_pitch / (fb_bpp / 8);
    const idx = @as(usize, y) * row_words + @as(usize, x);
    fb_addr[idx] = color;
}

/// 填充指定矩形区域为纯色
/// 自动裁剪到帧缓冲边界，不会越界写入
export fn hal_fb_fill_rect(x: u32, y: u32, w: u32, h: u32, color: u32) void {
    if (!fb_initialized) return;
    const row_words = fb_pitch / (fb_bpp / 8);

    // 裁剪矩形到屏幕范围
    const clip_x = if (x >= fb_width) fb_width else x;
    const clip_y = if (y >= fb_height) fb_height else y;
    const clip_w = if (clip_x + w > fb_width) fb_width - clip_x else w;
    const clip_h = if (clip_y + h > fb_height) fb_height - clip_y else h;

    var row: u32 = 0;
    while (row < clip_h) : (row += 1) {
        const base = @as(usize, clip_y + row) * row_words + @as(usize, clip_x);
        var col: u32 = 0;
        while (col < clip_w) : (col += 1) {
            fb_addr[base + col] = color;
        }
    }
}

/// 以指定颜色清空整个帧缓冲
export fn hal_fb_clear(color: u32) void {
    if (!fb_initialized) return;
    const row_words = fb_pitch / (fb_bpp / 8);
    const total = @as(usize, fb_height) * row_words;
    var i: usize = 0;
    while (i < total) : (i += 1) {
        fb_addr[i] = color;
    }
}

/// 将源图像数据块传输到帧缓冲指定位置
/// src 为紧凑排列的像素数据 (每像素 bpp/8 字节)，无行间填充
/// 自动裁剪超出屏幕的部分
export fn hal_fb_blit(src: [*]const u8, x: u32, y: u32, w: u32, h: u32) void {
    if (!fb_initialized) return;
    const row_words = fb_pitch / (fb_bpp / 8);
    const bytes_per_pixel = fb_bpp / 8;

    // 裁剪
    const clip_x = if (x >= fb_width) fb_width else x;
    const clip_y = if (y >= fb_height) fb_height else y;
    const clip_w = if (clip_x + w > fb_width) fb_width - clip_x else w;
    const clip_h = if (clip_y + h > fb_height) fb_height - clip_y else h;

    // 计算源数据中因裁剪产生的 x 偏移
    const src_x_offset = clip_x - x;

    var row: u32 = 0;
    while (row < clip_h) : (row += 1) {
        const src_y_offset = clip_y - y + row;
        const src_row_start = @as(usize, src_y_offset) * @as(usize, w) * bytes_per_pixel +
            @as(usize, src_x_offset) * bytes_per_pixel;
        const dst_base = @as(usize, clip_y + row) * row_words + @as(usize, clip_x);

        var col: u32 = 0;
        while (col < clip_w) : (col += 1) {
            const pixel_offset = src_row_start + @as(usize, col) * bytes_per_pixel;
            // 按字节拼装像素值，适配 32bpp 小端序
            const b = @as(u32, src[pixel_offset]);
            const g = @as(u32, src[pixel_offset + 1]);
            const r = @as(u32, src[pixel_offset + 2]);
            const pixel = (r << 16) | (g << 8) | b;
            fb_addr[dst_base + col] = pixel;
        }
    }
}

/// 获取当前帧缓冲的参数信息
/// 返回 FbInfo 结构体，未初始化时所有字段为零
export fn hal_fb_get_info() FbInfo {
    return FbInfo{
        .addr = @intFromPtr(fb_addr),
        .width = fb_width,
        .height = fb_height,
        .pitch = fb_pitch,
        .bpp = fb_bpp,
    };
}

/// 向上滚动指定像素行数，底部空出区域填充黑色
/// lines 为滚动的像素行数 (非字符行数)
export fn hal_fb_scroll(lines: u32) void {
    if (!fb_initialized) return;
    if (lines == 0 or lines >= fb_height) {
        // 滚动行数大于等于屏幕高度，等效于清屏
        hal_fb_clear(0x000000);
        return;
    }

    const row_words = fb_pitch / (fb_bpp / 8);
    const total_per_row = @as(usize, row_words);

    // 将第 lines 行起的内容逐行上移到第 0 行
    var row: u32 = 0;
    const move_rows = fb_height - lines;
    while (row < move_rows) : (row += 1) {
        const dst = @as(usize, row) * total_per_row;
        const src = @as(usize, row + lines) * total_per_row;
        var col: usize = 0;
        while (col < total_per_row) : (col += 1) {
            fb_addr[dst + col] = fb_addr[src + col];
        }
    }

    // 将底部 lines 行填充为黑色
    var clear_row: u32 = move_rows;
    while (clear_row < fb_height) : (clear_row += 1) {
        const base = @as(usize, clear_row) * total_per_row;
        var col: usize = 0;
        while (col < total_per_row) : (col += 1) {
            fb_addr[base + col] = 0x000000;
        }
    }
}
