// devices/input/usb/mod.rs — USB 子系统核心
//
// 主机控制器驱动 + 设备枚举 + HID 键盘驱动。
// 当前实现:
//   - xHCI (USB 3.x) 主机控制器 — 主力
//   - 通过 PCI 枚举发现控制器
//   - 简化初始化 + 端口轮询
//   - HID Boot Protocol 键盘支持
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod xhci;
pub mod hid;

use crate::serial;

/* ── USB 标准常量 ── */

/// USB 请求方向: 设备到主机
pub const REQ_DIR_IN: u8     = 0x80;
/// USB 请求方向: 主机到设备
pub const REQ_DIR_OUT: u8    = 0x00;
/// USB 请求类型: 标准
pub const REQ_TYPE_STD: u8   = 0x00;
/// USB 请求类型: 类
pub const REQ_TYPE_CLASS: u8 = 0x20;
/// USB 请求接收方: 设备
pub const REQ_RECIP_DEV: u8  = 0x00;
/// USB 请求接收方: 接口
pub const REQ_RECIP_IF: u8   = 0x01;

/// USB 标准请求: Get_Descriptor
pub const REQ_GET_DESCRIPTOR: u8 = 0x06;
/// USB 标准请求: Set_Configuration
pub const REQ_SET_CONFIG: u8     = 0x09;
/// USB 标准请求: Set_Interface
pub const REQ_SET_INTERFACE: u8  = 0x0B;

/// USB 描述符类型: Device
pub const DESC_DEVICE: u8  = 1;
/// USB 描述符类型: Configuration
pub const DESC_CONFIG: u8  = 2;
/// USB 描述符类型: String
pub const DESC_STRING: u8  = 3;
/// USB 描述符类型: Interface
pub const DESC_INTERFACE: u8 = 4;
/// USB 描述符类型: Endpoint
pub const DESC_ENDPOINT: u8 = 5;
/// USB 描述符类型: HID
pub const DESC_HID: u8     = 0x21;

/// USB 类代码: HID
pub const CLASS_HID: u8    = 3;
/// USB 类代码: Hub
pub const CLASS_HUB: u8    = 9;

/// HID 子类: Boot Interface
pub const SUBCLASS_BOOT: u8 = 1;

/// HID 协议: Keyboard
pub const PROTO_KEYBOARD: u8 = 1;

/* ── USB 设备描述符 ── */

/// USB 设备描述符 (18 字节)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DeviceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub bcd_usb: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub manufacturer: u8,
    pub product: u8,
    pub serial_number: u8,
    pub num_configurations: u8,
}

/// USB 配置描述符 (9 字节)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ConfigDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub total_length: u16,
    pub num_interfaces: u8,
    pub config_value: u8,
    pub config_string: u8,
    pub attributes: u8,
    pub max_power: u8,
}

/// USB 接口描述符 (9 字节)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct InterfaceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub num_endpoints: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
    pub interface_string: u8,
}

/// USB 端点描述符 (7 字节)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EndpointDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub endpoint_address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

/* ── 全局 USB 状态 ── */

/// 是否已初始化 USB 子系统
static mut USB_INITIALIZED: bool = false;

/// 找到的 USB 键盘数量
static mut KEYBOARD_COUNT: usize = 0;

/* ── 初始化 ── */

/// 初始化 USB 子系统 (xHCI 控制器发现 -> 端口枚举 -> HID 键盘扫描)
pub fn init() {
    unsafe {
        if USB_INITIALIZED {
            return;
        }

        serial::write_str(b"usb: probing USB controllers...\n");

        // 通过 PCI 枚举查找 xHCI 控制器
        let found = xhci::probe_controllers();
        if found > 0 {
            serial::write_str(b"usb: ");
            crate::serial_put_u64(found as u64);
            serial::write_str(b" xHCI controller(s) found\n");

            // 初始化第一个控制器
            if xhci::init_controller(0) {
                serial::write_str(b"usb: xHCI controller initialized\n");

                // 端口枚举
                xhci::enumerate_ports();

                // 扫描 HID 键盘
                hid::scan_keyboards();
            }
        } else {
            serial::write_str(b"usb: no xHCI controller found\n");
        }

        USB_INITIALIZED = true;
    }
}

/// 检查 USB 键盘是否可用
pub fn keyboard_available() -> bool {
    unsafe { KEYBOARD_COUNT > 0 }
}
