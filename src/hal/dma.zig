// dma.zig — ISA DMA 控制器 (8237) 抽象
// 管理 8 个 DMA 通道的分配、传输配置和启停
// 通道 0-3 为 8 位传输，通道 4-7 为 16 位传输 (通道 4 保留给级联)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

// 通道数量
const DMA_CHANNELS: u8 = 8;

// 传输方向常量
pub const DMA_DIR_READ: u8 = 0; // 内存到设备
pub const DMA_DIR_WRITE: u8 = 1; // 设备到内存

// DMA 控制器 1 (通道 0-3) I/O 端口
const DMA1_STATUS: u16 = 0x08; // 状态寄存器
const DMA1_CMD: u16 = 0x08; // 命令寄存器
const DMA1_REQUEST: u16 = 0x09; // 请求寄存器
const DMA1_MASK_SINGLE: u16 = 0x0A; // 单通道屏蔽
const DMA1_MODE: u16 = 0x0B; // 模式寄存器
const DMA1_FLIP_FLOP: u16 = 0x0C; // 触发器复位
const DMA1_MASK_ALL: u16 = 0x0F; // 全通道屏蔽

// DMA 控制器 2 (通道 4-7) I/O 端口
const DMA2_STATUS: u16 = 0xD0;
const DMA2_CMD: u16 = 0xD0;
const DMA2_REQUEST: u16 = 0xD2;
const DMA2_MASK_SINGLE: u16 = 0xD4;
const DMA2_MODE: u16 = 0xD6;
const DMA2_FLIP_FLOP: u16 = 0xD8;
const DMA2_MASK_ALL: u16 = 0xDE;

/// 每个通道的地址和计数寄存器端口
/// 地址寄存器: 通道 0=0x00, 1=0x02, 2=0x04, 3=0x06, 5=0xC4, 6=0xC8, 7=0xCC
/// 计数寄存器: 通道 0=0x01, 1=0x03, 2=0x05, 3=0x07, 5=0xC6, 6=0xCA, 7=0xCE
const ADDR_PORTS = [_]u16{ 0x00, 0x02, 0x04, 0x06, 0x00, 0xC4, 0xC8, 0xCC };
const COUNT_PORTS = [_]u16{ 0x01, 0x03, 0x05, 0x07, 0x00, 0xC6, 0xCA, 0xCE };

/// 页面寄存器端口 (用于提供地址的高 8 位)
const PAGE_PORTS = [_]u16{ 0x87, 0x83, 0x81, 0x82, 0x00, 0x8B, 0x89, 0x8A };

// 通道分配状态位图 (bit=1 表示已分配)
var channel_bitmap: u8 = 0;

// 各通道传输大小记录 (用于进度查询)
var channel_sizes: [DMA_CHANNELS]u32 = .{0} ** DMA_CHANNELS;

/// 向指定端口写入 8 位 I/O 数据
fn port_out8(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

/// 从指定端口读取 8 位 I/O 数据
fn port_in8(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}

/// 分配一个可用的 DMA 通道
/// 返回通道号 (0-7)，通道 4 (级联) 跳过，无可用通道时返回 -1
export fn hal_dma_alloc_channel() i32 {
    var ch: u8 = 0;
    while (ch < DMA_CHANNELS) : (ch += 1) {
        // 通道 4 保留给级联，不可分配
        if (ch == 4) continue;
        const bit: u8 = @as(u8, 1) << @intCast(ch);
        if ((channel_bitmap & bit) == 0) {
            channel_bitmap |= bit;
            // 屏蔽该通道防止意外传输
            mask_channel(ch, true);
            return @intCast(ch);
        }
    }
    return -1;
}

/// 释放已分配的 DMA 通道
/// 停止该通道上的传输并清除分配标记
export fn hal_dma_free_channel(ch: u8) void {
    if (ch >= DMA_CHANNELS) return;
    // 停止传输
    mask_channel(ch, true);
    // 清除分配标记
    const bit: u8 = @as(u8, 1) << @intCast(ch);
    channel_bitmap &= ~bit;
    channel_sizes[ch] = 0;
}

/// 配置 DMA 通道的传输参数
/// ch: 通道号 (0-3 为 8 位，5-7 为 16 位)
/// addr: 物理内存地址 (需低于 16MB)
/// size: 传输字节数
/// direction: 0=内存到设备 (读), 1=设备到内存 (写)
export fn hal_dma_setup_transfer(ch: u8, addr: u64, size: u32, direction: u8) void {
    if (ch >= DMA_CHANNELS or ch == 4) return;

    // 屏蔽通道
    mask_channel(ch, true);

    // 复位触发器 (确保后续连续写高低字节正确)
    if (ch < 4) {
        port_out8(DMA1_FLIP_FLOP, 0);
    } else {
        port_out8(DMA2_FLIP_FLOP, 0);
    }

    // 设置模式寄存器
    // 模式位: [7:6]=传输类型 (01=校验, 10=写, 01=读, 00=校验)
    //          [5]=自动初始化, [4]=地址递减, [3:2]=通道模式, [1:0]=通道号
    // 单传输模式，地址递增
    var mode: u8 = ch & 0x03;
    if (direction == DMA_DIR_READ) {
        // 内存到设备: 模式位 [7:6] = 01 (写传输，从控制器角度看是写)
        mode |= 0x48; // 01_0_0_10_xx -> 单传输, 地址递增, 写模式
    } else {
        // 设备到内存: 模式位 [7:6] = 10 (读传输)
        mode |= 0x44; // 01_0_0_01_xx -> 单传输, 地址递增, 读模式
    }

    if (ch < 4) {
        port_out8(DMA1_MODE, mode);
    } else {
        port_out8(DMA2_MODE, mode);
    }

    // 计算实际地址和计数
    const phys_addr: u32 = @intCast(addr & 0xFFFFFF);
    const page: u8 = @intCast((phys_addr >> 16) & 0xFF);

    // 16 位通道需要将地址和计数除以 2
    var base_addr: u32 = phys_addr;
    var count: u32 = size;
    if (ch >= 5) {
        base_addr = phys_addr / 2;
        count = size / 2;
    }

    // 写入地址寄存器 (先低后高)
    const addr_port = ADDR_PORTS[ch];
    port_out8(addr_port, @intCast(base_addr & 0xFF));
    port_out8(addr_port, @intCast((base_addr >> 8) & 0xFF));

    // 写入页面寄存器
    if (PAGE_PORTS[ch] != 0) {
        port_out8(PAGE_PORTS[ch], page);
    }

    // 写入计数寄存器 (传输字节数 - 1，先低后高)
    const count_val: u32 = if (count > 0) count - 1 else 0;
    const count_port = COUNT_PORTS[ch];
    port_out8(count_port, @intCast(count_val & 0xFF));
    port_out8(count_port, @intCast((count_val >> 8) & 0xFF));

    // 记录传输大小供进度查询
    channel_sizes[ch] = size;
}

/// 启动指定 DMA 通道的传输
/// 取消通道屏蔽，允许 DMA 控制器开始操作
export fn hal_dma_start(ch: u8) void {
    if (ch >= DMA_CHANNELS or ch == 4) return;
    mask_channel(ch, false);
}

/// 停止指定 DMA 通道的传输
/// 屏蔽通道以终止正在进行的 DMA 操作
export fn hal_dma_stop(ch: u8) void {
    if (ch >= DMA_CHANNELS or ch == 4) return;
    mask_channel(ch, true);
}

/// 查询指定 DMA 通道的传输进度
/// 返回已传输的字节数，通过读取当前计数寄存器计算
export fn hal_dma_get_progress(ch: u8) u32 {
    if (ch >= DMA_CHANNELS or ch == 4) return 0;

    const total = channel_sizes[ch];
    if (total == 0) return 0;

    // 复位触发器
    if (ch < 4) {
        port_out8(DMA1_FLIP_FLOP, 0);
    } else {
        port_out8(DMA2_FLIP_FLOP, 0);
    }

    // 读取当前剩余计数 (先低后高)
    const count_port = COUNT_PORTS[ch];
    const lo: u16 = port_in8(count_port);
    const hi: u16 = port_in8(count_port);
    const remaining: u32 = (@as(u32, hi) << 8) | lo;

    // 16 位通道计数值需乘以 2
    var remaining_bytes: u32 = remaining + 1;
    if (ch >= 5) {
        remaining_bytes *= 2;
    }

    // 已传输字节 = 总大小 - 剩余
    if (remaining_bytes >= total) return total;
    return total - remaining_bytes;
}

/// 屏蔽或取消屏蔽指定 DMA 通道
/// mask=true 屏蔽 (禁用)，mask=false 取消屏蔽 (启用)
fn mask_channel(ch: u8, mask: bool) void {
    if (ch < 4) {
        var val: u8 = ch & 0x03;
        if (mask) val |= 0x04; // 位 2 置 1 表示屏蔽
        port_out8(DMA1_MASK_SINGLE, val);
    } else {
        var val: u8 = (ch - 4) & 0x03;
        if (mask) val |= 0x04;
        port_out8(DMA2_MASK_SINGLE, val);
    }
}
