// devices/mod.rs — 设备驱动框架核心
//
// 管理设备树、PCI 总线、驱动注册与探针。
// 所有硬件设备在内核启动时通过 PCI 枚举和固定设备初始化完成发现。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod pci;
pub mod input;

use core::cell::UnsafeCell;

/* ── 设备类型 ── */

/// 设备分类枚举
#[repr(u8)]
pub enum DeviceClass {
    Pci = 0,
    Virtio = 1,
    Block = 2,
    Net = 3,
    Display = 4,
    Input = 5,
    Audio = 6,
    Serial = 7,
    Unknown = 0xFF,
}

/* ── MMIO / I/O 资源 ── */

/// 设备资源描述 (I/O 端口或 MMIO 区域)
#[repr(C)]
pub struct Resource {
    /// 资源类型: 0=IO port, 1=MMIO
    pub kind: u8,
    pub start: u64,
    pub end: u64,
    pub flags: u32,
}

/* ── 设备节点 ── */

/// 内核设备树节点 (侵入式链表结构)
#[repr(C)]
pub struct Device {
    pub class: DeviceClass,
    pub name: &'static str,
    pub irq: u8,
    pub resources: &'static [Resource],
    /// 驱动私有数据
    pub priv_data: *mut (),
    /// 父设备
    pub parent: *mut Device,
    /// 子设备链表头
    pub children: *mut Device,
    /// 兄弟设备链表
    pub next: *mut Device,
}

impl Device {
    /// 创建设备节点
    pub const fn new(name: &'static str, class: DeviceClass) -> Self {
        Device {
            class,
            name,
            irq: 0,
            resources: &[],
            priv_data: core::ptr::null_mut(),
            parent: core::ptr::null_mut(),
            children: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        }
    }
}

/* ── 驱动接口 ── */

/// 驱动必须实现的接口
pub trait Driver: Sync {
    /// 驱动名称
    fn name(&self) -> &'static str;
    /// 探针函数: 检测设备是否匹配
    fn probe(&self, dev: &Device) -> bool;
    /// 初始化设备
    fn init(&self, dev: &mut Device) -> Result<(), ()>;
}

/* ── 设备树管理器 ── */

/// 设备树: 以根节点为起点的设备层次结构
pub struct DeviceTree {
    root: Device,
    device_count: usize,
}

impl DeviceTree {
    /// 创建空设备树
    pub const fn new() -> Self {
        DeviceTree {
            root: Device::new("root", DeviceClass::Unknown),
            device_count: 0,
        }
    }

    /// 注册设备到树中 (作为根节点的子设备)
    pub fn add_device(&mut self, dev: &'static mut Device) {
        dev.parent = &mut self.root as *mut Device;
        dev.next = self.root.children;
        self.root.children = dev;
        self.device_count += 1;
    }

    /// 获取已注册设备数量
    pub fn device_count(&self) -> usize {
        self.device_count
    }
}

/* ── 全局设备树 ── */

struct DevTreeWrapper(UnsafeCell<DeviceTree>);
unsafe impl Sync for DevTreeWrapper {}

static DEV_TREE: DevTreeWrapper = DevTreeWrapper(UnsafeCell::new(DeviceTree::new()));

/// 初始化设备子系统: PCI 枚举 + 输入设备
pub fn init() {
    crate::serial::write_str(b"  dev: pci enum start\n");
    pci::enumerate_all();
    crate::serial::write_str(b"  dev: pci enum done (");
    crate::serial_put_u64(pci::device_count() as u64);
    crate::serial::write_str(b" devs)\n");
    crate::serial::write_str(b"  dev: input init start\n");
    input::init();
    crate::serial::write_str(b"  dev: input init done\n");
}

/// 注册设备到全局设备树
pub fn add_device(dev: &'static mut Device) {
    unsafe { (*DEV_TREE.0.get()).add_device(dev); }
}

/// 获取已注册设备数量
pub fn device_count() -> usize {
    unsafe { (*DEV_TREE.0.get()).device_count() }
}
