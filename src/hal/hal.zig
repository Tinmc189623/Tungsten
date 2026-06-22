// hal.zig — Tungsten 硬件抽象层根模块
// 聚合所有 HAL 子模块，统一对外暴露硬件抽象接口
// 编译目标: x86_64-freestanding
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

pub const ioport = @import("ioport.zig");
pub const serial = @import("serial.zig");
pub const framebuffer = @import("framebuffer.zig");
pub const pci = @import("pci.zig");
pub const timer = @import("timer.zig");
pub const interrupt = @import("interrupt.zig");
pub const dma = @import("dma.zig");
