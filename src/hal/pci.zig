// pci.zig — PCI 配置空间访问与总线扫描
// 通过 I/O 端口 0xCF8/0xCFC (CONFIG_ADDRESS / CONFIG_DATA) 实现 Type 1 配置访问
// Copyright (C) 2026 Nexsteaduser. All rights reserved.

// PCI 配置机制 I/O 端口
const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// PCI 设备信息结构，用于总线扫描结果返回
pub const PciDeviceInfo = extern struct {
    /// 厂商标识 (0xFFFF 表示设备不存在)
    vendor_id: u16,
    /// 设备标识
    device_id: u16,
    /// 设备大类代码
    class_code: u8,
    /// 设备子类代码
    subclass: u8,
    /// 所在总线号
    bus: u8,
    /// 设备号 (0-31)
    dev: u8,
    /// 功能号 (0-7)
    func: u8,
    /// 填充字节，保证对齐
    _pad: u8 = 0,
    /// 基地址寄存器 (最多 6 个)
    bar: [6]u32,
};

/// 向指定端口写入 32 位 I/O 数据
fn port_out32(port: u16, val: u32) void {
    asm volatile ("outl %[val], %[port]"
        :
        : [val] "{eax}" (val),
          [port] "{dx}" (port),
    );
}

/// 从指定端口读取 32 位 I/O 数据
fn port_in32(port: u16) u32 {
    return asm volatile ("inl %[port], %[ret]"
        : [ret] "={eax}" (-> u32),
        : [port] "{dx}" (port),
    );
}

/// 构造 PCI Type 1 配置地址
/// 位 31: 使能位 | 位 23-16: 总线 | 位 15-11: 设备 | 位 10-8: 功能 | 位 7-2: 寄存器偏移
fn make_address(bus: u8, dev: u8, func: u8, offset: u8) u32 {
    return (@as(u32, 1) << 31) |
        (@as(u32, bus) << 16) |
        (@as(u32, dev & 0x1F) << 11) |
        (@as(u32, func & 0x07) << 8) |
        (@as(u32, offset) & 0xFC);
}

/// 读取 PCI 配置空间中指定总线/设备/功能的 32 位寄存器
/// offset 自动对齐到 4 字节边界
export fn hal_pci_read_config(bus: u8, dev: u8, func: u8, offset: u8) u32 {
    const addr = make_address(bus, dev, func, offset);
    port_out32(PCI_CONFIG_ADDRESS, addr);
    return port_in32(PCI_CONFIG_DATA);
}

/// 向 PCI 配置空间中指定总线/设备/功能的 32 位寄存器写入值
/// offset 自动对齐到 4 字节边界
export fn hal_pci_write_config(bus: u8, dev: u8, func: u8, offset: u8, val: u32) void {
    const addr = make_address(bus, dev, func, offset);
    port_out32(PCI_CONFIG_ADDRESS, addr);
    port_out32(PCI_CONFIG_DATA, val);
}

/// 读取指定设备的 BAR 寄存器
/// header_type 为设备头部类型 (0=标准设备, 1=PCI-PCI 桥, 2=CardBus 桥)
/// 标准设备有 6 个 BAR，桥接设备有 2 个
fn read_bars(bus: u8, dev: u8, func: u8, header_type: u8) [6]u32 {
    var bars: [6]u32 = .{0} ** 6;
    const bar_count: u8 = if (header_type == 0) 6 else 2;
    var i: u8 = 0;
    while (i < bar_count) : (i += 1) {
        // BAR0 位于配置空间偏移 0x10，每个 BAR 占 4 字节
        const bar_offset: u8 = 0x10 + i * 4;
        bars[i] = hal_pci_read_config(bus, dev, func, bar_offset);
    }
    return bars;
}

/// 扫描指定 PCI 总线，将发现的设备信息写入 devices 数组
/// 返回实际发现的设备数量，不超过 max
/// 跳过 vendor_id 为 0xFFFF 的空槽位，同时检测多功能设备
export fn hal_pci_scan_bus(bus: u8, devices: [*]PciDeviceInfo, max: usize) usize {
    var count: usize = 0;
    var dev: u8 = 0;

    while (dev < 32) : (dev += 1) {
        // 先检查功能 0
        const id_reg = hal_pci_read_config(bus, dev, 0, 0x00);
        const vendor_id: u16 = @intCast(id_reg & 0xFFFF);

        if (vendor_id == 0xFFFF) {
            // 该槽位无设备
            continue;
        }

        if (count < max) {
            const info = read_device_info(bus, dev, 0, id_reg);
            devices[count] = info;
            count += 1;
        }

        // 检查是否为多功能设备 (头部类型寄存器 bit 7)
        const header_reg = hal_pci_read_config(bus, dev, 0, 0x0C);
        const multifunction = (header_reg & 0x00800000) != 0;

        if (multifunction) {
            var func: u8 = 1;
            while (func < 8) : (func += 1) {
                const func_id = hal_pci_read_config(bus, dev, func, 0x00);
                const func_vendor: u16 = @intCast(func_id & 0xFFFF);
                if (func_vendor == 0xFFFF) {
                    continue;
                }
                if (count < max) {
                    const info = read_device_info(bus, dev, func, func_id);
                    devices[count] = info;
                    count += 1;
                }
            }
        }
    }

    return count;
}

/// 从配置空间读取单个设备的完整信息
fn read_device_info(bus: u8, dev: u8, func: u8, id_reg: u32) PciDeviceInfo {
    const vendor_id: u16 = @intCast(id_reg & 0xFFFF);
    const device_id: u16 = @intCast((id_reg >> 16) & 0xFFFF);

    // 读取类代码寄存器 (偏移 0x08): 位 31-24=class, 23-16=subclass, 15-8=prog_if
    const class_reg = hal_pci_read_config(bus, dev, func, 0x08);
    const class_code: u8 = @intCast((class_reg >> 24) & 0xFF);
    const subclass: u8 = @intCast((class_reg >> 16) & 0xFF);

    // 读取头部类型 (偏移 0x0C 的位 23-16)
    const header_reg = hal_pci_read_config(bus, dev, func, 0x0C);
    const header_type: u8 = @intCast((header_reg >> 16) & 0xFF);

    const bars = read_bars(bus, dev, func, header_type);

    return PciDeviceInfo{
        .vendor_id = vendor_id,
        .device_id = device_id,
        .class_code = class_code,
        .subclass = subclass,
        .bus = bus,
        .dev = dev,
        .func = func,
        .bar = bars,
    };
}
