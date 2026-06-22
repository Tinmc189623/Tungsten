// net/mod.rs — Tungsten 网络子系统
// TCP/IP 协议栈、套接字接口、网络设备驱动框架
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod socket;
pub mod ip;
pub mod tcp;
pub mod udp;
pub mod ethernet;
pub mod arp;
pub mod icmp;
pub mod dns;
pub mod dhcp;

use crate::sync::SpinLock;

/* ── 网络设备接口 ── */

#[repr(C)]
pub struct NetDevice {
    pub name: [u8; 16],
    pub mac: [u8; 6],
    pub mtu: u16,
    pub flags: u32,
    pub rx_queue: SpinLock<NetRxQueue>,
    pub tx_queue: SpinLock<NetTxQueue>,
    pub stats: NetStats,
    pub ops: &'static NetDeviceOps,
    pub priv_data: *mut (),
    pub next: *mut NetDevice,
}

#[repr(C)]
pub struct NetDeviceOps {
    pub open: unsafe extern "C" fn(dev: *mut NetDevice) -> i32,
    pub stop: unsafe extern "C" fn(dev: *mut NetDevice),
    pub xmit: unsafe extern "C" fn(dev: *mut NetDevice, data: *const u8, len: usize) -> i32,
    pub set_mac: unsafe extern "C" fn(dev: *mut NetDevice, mac: *const u8),
    pub ioctl: unsafe extern "C" fn(dev: *mut NetDevice, cmd: u32, arg: *mut ()) -> i32,
}

/* ── 网络数据包缓冲 ── */

pub const NET_PKT_MAX: usize = 2048;
pub const NET_RX_RING_SIZE: usize = 64;
pub const NET_TX_RING_SIZE: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetPacket {
    pub data: [u8; NET_PKT_MAX],
    pub len: u16,
    pub flags: u16,
    pub protocol: u16,
    pub timestamp: u64,
}

impl NetPacket {
    pub const fn new() -> Self {
        NetPacket { data: [0u8; NET_PKT_MAX], len: 0, flags: 0, protocol: 0, timestamp: 0 }
    }
}

/* ── 网络队列 ── */

pub struct NetRxQueue {
    pub packets: [NetPacket; NET_RX_RING_SIZE],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
}

impl NetRxQueue {
    pub const fn new() -> Self {
        NetRxQueue {
            packets: [NetPacket::new(); NET_RX_RING_SIZE],
            head: 0, tail: 0, count: 0,
        }
    }
    pub fn enqueue(&mut self, pkt: NetPacket) -> bool {
        if self.count >= NET_RX_RING_SIZE { return false; }
        self.packets[self.tail] = pkt;
        self.tail = (self.tail + 1) % NET_RX_RING_SIZE;
        self.count += 1;
        true
    }
    pub fn dequeue(&mut self) -> Option<NetPacket> {
        if self.count == 0 { return None; }
        let pkt = self.packets[self.head].clone();
        self.head = (self.head + 1) % NET_RX_RING_SIZE;
        self.count -= 1;
        Some(pkt)
    }
}

pub struct NetTxQueue {
    pub packets: [NetPacket; NET_TX_RING_SIZE],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
}

impl NetTxQueue {
    pub const fn new() -> Self {
        NetTxQueue {
            packets: [NetPacket::new(); NET_TX_RING_SIZE],
            head: 0, tail: 0, count: 0,
        }
    }
    pub fn enqueue(&mut self, pkt: NetPacket) -> bool {
        if self.count >= NET_TX_RING_SIZE { return false; }
        self.packets[self.tail] = pkt;
        self.tail = (self.tail + 1) % NET_TX_RING_SIZE;
        self.count += 1;
        true
    }
}

/* ── 网络统计 ── */

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NetStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
}

/* ── 全局网络管理 ── */

pub struct NetManager {
    pub devices: *mut NetDevice,
    pub device_count: usize,
    pub initialized: bool,
}
unsafe impl Send for NetManager {}

static NET_MANAGER: SpinLock<NetManager> = SpinLock::new(NetManager {
    devices: core::ptr::null_mut(),
    device_count: 0,
    initialized: false,
});

pub fn init() {
    crate::serial::write_str(b"net: initializing network subsystem...\n");
    let mut mgr = NET_MANAGER.lock();
    mgr.initialized = true;
    crate::serial::write_str(b"net: network subsystem ready\n");
}

pub fn register_device(dev: &'static mut NetDevice) -> i32 {
    let mut mgr = NET_MANAGER.lock();
    dev.next = mgr.devices;
    mgr.devices = dev;
    mgr.device_count += 1;
    crate::serial::write_str(b"net: registered device ");
    crate::serial::write_str(&dev.name);
    crate::serial::write_str(b"\n");
    0
}

pub fn device_count() -> usize {
    NET_MANAGER.lock().device_count
}

/// 网络服务周期轮询（由 netd 调用）
pub fn poll() {
    let _mgr = NET_MANAGER.lock();
    // 协议栈定时器与 ARP 老化在设备驱动就绪后扩展
}
