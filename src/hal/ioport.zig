// ioport.zig — x86_64 I/O 端口读写原语
// 封装 in/out 指令，向 Rust 内核提供 8/16/32 位端口访问及块传输
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

/// 从指定端口读取 8 位数据
export fn hal_ioport_read8(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}

/// 从指定端口读取 16 位数据
export fn hal_ioport_read16(port: u16) u16 {
    return asm volatile ("inw %[port], %[ret]"
        : [ret] "={ax}" (-> u16),
        : [port] "{dx}" (port),
    );
}

/// 从指定端口读取 32 位数据
export fn hal_ioport_read32(port: u16) u32 {
    return asm volatile ("inl %[port], %[ret]"
        : [ret] "={eax}" (-> u32),
        : [port] "{dx}" (port),
    );
}

/// 向指定端口写入 8 位数据
export fn hal_ioport_write8(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

/// 向指定端口写入 16 位数据
export fn hal_ioport_write16(port: u16, val: u16) void {
    asm volatile ("outw %[val], %[port]"
        :
        : [val] "{ax}" (val),
          [port] "{dx}" (port),
    );
}

/// 向指定端口写入 32 位数据
export fn hal_ioport_write32(port: u16, val: u32) void {
    asm volatile ("outl %[val], %[port]"
        :
        : [val] "{eax}" (val),
          [port] "{dx}" (port),
    );
}

/// 使用 REP INSW 从端口批量读取 16 位数据到缓冲区
/// count 为要读取的 u16 元素数量
export fn hal_ioport_read_block(port: u16, buf: [*]u16, count: usize) void {
    asm volatile (
        \\cld
        \\rep insw
        :
        : [port] "{dx}" (port),
          [count] "{ecx}" (count),
          [buf] "{rdi}" (buf),
    );
}

/// 使用 REP OUTSW 将缓冲区中的 16 位数据批量写入端口
/// count 为要写入的 u16 元素数量
export fn hal_ioport_write_block(port: u16, buf: [*]const u16, count: usize) void {
    asm volatile (
        \\cld
        \\rep outsw
        :
        : [port] "{dx}" (port),
          [count] "{ecx}" (count),
          [buf] "{rsi}" (buf),
    );
}

// ── Rust 兼容层（短名称） ──

/// 读取 8 位端口 (hal_inb)
export fn hal_inb(port: u16) u8 {
    return hal_ioport_read8(port);
}

/// 写入 8 位端口 (hal_outb)
export fn hal_outb(port: u16, val: u8) void {
    hal_ioport_write8(port, val);
}

/// 读取 16 位端口 (hal_inw)
export fn hal_inw(port: u16) u16 {
    return hal_ioport_read16(port);
}

/// 写入 16 位端口 (hal_outw)
export fn hal_outw(port: u16, val: u16) void {
    hal_ioport_write16(port, val);
}

/// 读取 32 位端口 (hal_ind)
export fn hal_ind(port: u16) u32 {
    return hal_ioport_read32(port);
}

/// 写入 32 位端口 (hal_outd)
export fn hal_outd(port: u16, val: u32) void {
    hal_ioport_write32(port, val);
}
