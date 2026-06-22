// devices/input/usb/xhci.rs — xHCI (USB 3.x) 主机控制器驱动
//
// 简化实现: PCI 发现 -> MMIO 映射 -> 端口枚举 -> 设备配置
// 仅实现 HID Boot Protocol 键盘所需的通路。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::serial;
use crate::devices::pci;

/* ── xHCI PCI 类别 ── */

const PCI_CLASS_SERIAL: u8     = 0x0C;
const PCI_SUBCLASS_USB: u8     = 0x03;
const PCI_PROGIF_XHCI: u8     = 0x30;

/* ── xHCI MMIO 寄存器偏移 (CAPABILITY) ── */

const CAPLENGTH:  u8 = 0x00;
const HCIVERSION: u8 = 0x02;
const HCSPARAMS1: u8 = 0x04;
const HCSPARAMS2: u8 = 0x08;
const HCSPARAMS3: u8 = 0x0C;
const HCCPARAMS1: u8 = 0x10;

/// HCSPARAMS1 位域: 最大设备插槽数
fn max_slots(hcsp: u32) -> u8 {
    ((hcsp >> 0) & 0xFF) as u8
}

/// HCSPARAMS1 位域: 最大端口数
fn max_ports(hcsp: u32) -> u8 {
    ((hcsp >> 24) & 0xFF) as u8
}

/// HCCPARAMS1 位域: 扩展能力指针偏移
fn xecp_offset(hccp: u32) -> u16 {
    ((hccp >> 16) & 0xFFFF) as u16
}

/* ── xHCI OP 寄存器偏移 ── */

const USBSTS:   u32 = 0x00;
const USBCMD:   u32 = 0x04;
const DNCTRL:   u32 = 0x14;
const CRCR:     u32 = 0x18;
const DCBAAP:   u32 = 0x30;
const CONFIG:   u32 = 0x38;

/// USBSTS 位
const STS_HCH:   u32 = 1 << 0;
const STS_PCD:   u32 = 1 << 2;
const STS_EINT:  u32 = 1 << 3;

/// USBCMD 位
const CMD_RUN:   u32 = 1 << 0;
const CMD_HCRST: u32 = 1 << 1;
const CMD_INTE:  u32 = 1 << 2;

/// PORTSC 位
const PORTSC_CCS:        u32 = 1 << 0;
const PORTSC_PED:        u32 = 1 << 1;
const PORTSC_PR:         u32 = 1 << 4;
const PORTSC_PLS_SHIFT:  u32 = 5;
const PORTSC_PLS_MASK:   u32 = 0xF << 5;
const PORTSC_SPEED_SHIFT: u32 = 20;
const PORTSC_SPEED_MASK: u32 = 0xF << 20;

/// 端口链接状态
const PLS_POLLING: u32 = 7;
const PLS_RESUME:  u32 = 15;

/* ── TRB 类型 ── */

const TRB_CMD_NOOP:      u8 = 23;
const TRB_CMD_ENABLE_SLOT: u8 = 9;
const TRB_CMD_ADDRESS_DEVICE: u8 = 11;
const TRB_CMD_CONFIG_ENDPOINT: u8 = 12;
const TRB_CMD_EVAL_CONTEXT: u8 = 13;
const TRB_CMD_RESET_DEVICE: u8 = 14;

const TRB_TR_NORMAL:       u8 = 1;
const TRB_TR_SETUP_STAGE:  u8 = 2;
const TRB_TR_DATA_STAGE:   u8 = 3;
const TRB_TR_STATUS_STAGE: u8 = 4;

const TRB_EV_TRANSFER:     u8 = 32;
const TRB_EV_CMD_COMPLETE: u8 = 33;
const TRB_EV_PORT_STATUS:  u8 = 34;

/* ── xHCI 控制器状态 ── */

/// 发现的 xHCI 控制器数量
static mut CONTROLLER_COUNT: usize = 0;

/// xHCI 控制器 MMIO 地址
static mut MMIO_BASE: [u64; 4] = [0; 4];

/// 每个控制器的端口数
static mut PORT_COUNT: [u8; 4] = [0; 4];

/// 最大设备插槽数
static mut MAX_SLOTS: [u8; 4] = [0; 4];

/// 已分配的插槽位图
static mut SLOT_ALLOC: [u64; 4] = [0; 4];

/// 设备上下文基址数组
static mut DCBAA: [u64; 4] = [0; 4];

/// 设备速度 (slot -> speed)
static mut DEVICE_SPEED: [u8; 256] = [0; 256];

/* ── TRB 结构 ── */

/// 传输请求块 (16 字节, 64 字节对齐)
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Trb {
    params: [u32; 4],
}

impl Trb {
    /// 创建全零 TRB
    const fn zero() -> Self {
        Trb { params: [0; 4] }
    }
}

/* ── 事件 TRB ── */

#[repr(C)]
struct EventTrb {
    params: [u32; 4],
}

impl EventTrb {
    /// 获取 TRB 类型
    fn type_(&self) -> u8 {
        ((self.params[3] >> 10) & 0x3F) as u8
    }
    /// 获取完成码
    fn completion_code(&self) -> u8 {
        ((self.params[2] >> 24) & 0xFF) as u8
    }
    /// 获取设备插槽 ID
    fn slot_id(&self) -> u8 {
        ((self.params[3] >> 24) & 0xFF) as u8
    }
    /// 获取 TRB 指针
    fn trb_pointer(&self) -> u64 {
        self.params[0] as u64 | ((self.params[1] as u64) << 32)
    }
}

/* ── MMIO 访问 ── */

/// 读 MMIO 32 位
unsafe fn mmio_read32(base: u64, offset: u32) -> u32 {
    core::ptr::read_volatile((base + offset as u64) as *const u32)
}

/// 写 MMIO 32 位
unsafe fn mmio_write32(base: u64, offset: u32, val: u32) {
    core::ptr::write_volatile((base + offset as u64) as *mut u32, val);
}

/// 读 MMIO 64 位
unsafe fn mmio_read64(base: u64, offset: u32) -> u64 {
    core::ptr::read_volatile((base + offset as u64) as *const u64)
}

/// 写 MMIO 64 位
unsafe fn mmio_write64(base: u64, offset: u32, val: u64) {
    core::ptr::write_volatile((base + offset as u64) as *mut u64, val);
}

/// 读 MMIO 8 位
unsafe fn mmio_read8(base: u64, offset: u32) -> u8 {
    core::ptr::read_volatile((base + offset as u64) as *const u8)
}

/* ── 命令环 ── */

const CMD_RING_SIZE: usize = 32;

/// 命令环: TRB 数组 + 入队指针
struct CommandRing {
    trbs: [Trb; CMD_RING_SIZE],
    enqueue: usize,
    paddr: u64,
    doorbell: u64,
}

static mut CMD_RING: CommandRing = CommandRing {
    trbs: [Trb::zero(); CMD_RING_SIZE],
    enqueue: 0,
    paddr: 0,
    doorbell: 0,
};

/// 初始化命令环 (设置 CRCR 寄存器)
unsafe fn init_command_ring(base: u64, op_offset: u32) {
    let ring_paddr = core::ptr::addr_of!(CMD_RING.trbs) as u64;

    CMD_RING.paddr = ring_paddr;
    CMD_RING.enqueue = 0;
    CMD_RING.doorbell = base + op_offset as u64 + 0x00;

    // 写 CRCR: RCS = 1
    let crcr_value = ring_paddr | 1;
    mmio_write64(base, op_offset as u32 + CRCR as u32, crcr_value);

    serial::write_str(b"xhci: command ring at 0x");
    crate::serial_put_u64_hex(ring_paddr);
    serial::write_str(b"\n");
}

/// 向命令环写入一个 TRB
unsafe fn enqueue_command(trb: &Trb) -> bool {
    let idx = CMD_RING.enqueue;
    if idx >= CMD_RING_SIZE - 1 {
        return false;
    }
    CMD_RING.trbs[idx] = *trb;

    // 循环位: cycle bit = 1
    let cycle = 1u32;
    let old = CMD_RING.trbs[idx].params[3];
    CMD_RING.trbs[idx].params[3] = (old & !(1u32 << 0)) | cycle;

    // 内存屏障 + 门铃
    core::arch::asm!("mfence");
    mmio_write32(CMD_RING.doorbell as u64, 0, 0);

    CMD_RING.enqueue = (idx + 1) % CMD_RING_SIZE;
    true
}

/* ── 事件环 (简化: 轮询读事件) ── */

/// 等待并读取一个事件 TRB (简化: 忙等待 USBSTS)
unsafe fn wait_for_event(base: u64, op_offset: u32) -> Option<EventTrb> {
    for _ in 0..100000 {
        let usbsts = mmio_read32(base, op_offset as u32 + USBSTS as u32);
        if usbsts & STS_EINT != 0 {
            mmio_write32(base, op_offset as u32 + USBSTS as u32, STS_EINT);
            return None;
        }
        core::arch::asm!("pause");
    }
    None
}

/* ── 控制器发现 ── */

/// 在 PCI 总线上查找 xHCI 控制器
pub fn probe_controllers() -> usize {
    let count = pci::device_count();
    let mut found = 0usize;

    for i in 0..count {
        let dev = pci::devices()[i];

        if dev.class_code == PCI_CLASS_SERIAL
            && dev.subclass == PCI_SUBCLASS_USB
            && dev.prog_if == PCI_PROGIF_XHCI
        {
            if found < 4 {
                let bar0 = dev.bars[0];
                let bar1 = dev.bars[1];

                let mmio_base = if bar0 & 1 == 0 {
                    if bar0 & 0x4 != 0 {
                        (bar0 & 0xFFFFFFF0) as u64 | ((bar1 as u64) << 32)
                    } else {
                        (bar0 & 0xFFFFFFF0) as u64
                    }
                } else {
                    continue;
                };

                unsafe {
                    MMIO_BASE[found] = mmio_base;
                }
                found += 1;

                serial::write_str(b"xhci: controller at ");
                crate::serial_put_u64_hex(mmio_base);
                serial::write_str(b" (dev ");
                crate::serial_put_u64(dev.dev as u64);
                serial::write_str(b" func ");
                crate::serial_put_u64(dev.func as u64);
                serial::write_str(b")\n");
            }
        }
    }

    unsafe {
        CONTROLLER_COUNT = found;
    }
    found
}

/* ── 控制器初始化 ── */

/// 初始化第 idx 个 xHCI 控制器 (复位 -> 配置 -> 运行)
pub fn init_controller(idx: usize) -> bool {
    if idx >= unsafe { CONTROLLER_COUNT } {
        return false;
    }

    unsafe {
        let mmio = MMIO_BASE[idx];

        // 读 Capability 寄存器
        let caplength = mmio_read8(mmio, CAPLENGTH as u32);
        let hcsp1 = mmio_read32(mmio, HCSPARAMS1 as u32);

        let n_ports = max_ports(hcsp1);
        let n_slots = max_slots(hcsp1);
        PORT_COUNT[idx] = n_ports;
        MAX_SLOTS[idx] = n_slots;

        serial::write_str(b"xhci: ports=");
        crate::serial_put_u64(n_ports as u64);
        serial::write_str(b" slots=");
        crate::serial_put_u64(n_slots as u64);
        serial::write_str(b"\n");

        let op_offset = caplength as u32;

        // 复位控制器
        mmio_write32(mmio, op_offset + USBCMD, CMD_HCRST);
        for _ in 0..100000 {
            if mmio_read32(mmio, op_offset + USBCMD) & CMD_HCRST == 0 {
                break;
            }
            core::arch::asm!("pause");
        }

        // 设置 CONFIG: 最大设备插槽数
        mmio_write32(mmio, op_offset + CONFIG, n_slots as u32);

        // 初始化命令环
        init_command_ring(mmio, op_offset);

        // DCBAA: 设备上下文基址数组
        let dcbaa_addr = mmio + 0x10000;
        mmio_write64(mmio, op_offset + DCBAAP, dcbaa_addr);
        for i in 0..n_slots as u32 {
            core::ptr::write_volatile((dcbaa_addr + i as u64 * 8) as *mut u64, 0u64);
        }
        DCBAA[idx] = dcbaa_addr;

        // 运行控制器
        mmio_write32(mmio, op_offset + USBCMD, CMD_RUN | CMD_INTE);

        for _ in 0..10000 {
            if mmio_read32(mmio, op_offset + USBSTS) & STS_HCH == 0 {
                break;
            }
            core::arch::asm!("pause");
        }

        serial::write_str(b"xhci: controller running\n");
        true
    }
}

/* ── 端口枚举 ── */

/// 获取 PORTSC 寄存器偏移
unsafe fn portsc_addr(_base: u64, op_offset: u32, port: u8) -> u32 {
    op_offset + 0x400 + (port as u32 - 1) * 16
}

/// 枚举所有端口，检测已连接的设备并执行端口复位
pub fn enumerate_ports() {
    unsafe {
        for ci in 0..CONTROLLER_COUNT {
            let mmio = MMIO_BASE[ci];
            let caplength = mmio_read8(mmio, CAPLENGTH as u32);
            let op_offset = caplength as u32;
            let n_ports = PORT_COUNT[ci];

            serial::write_str(b"xhci: enumerating ports...\n");

            for port in 1..=n_ports {
                let psc_addr = portsc_addr(mmio, op_offset, port);
                let portsc = mmio_read32(mmio, psc_addr);

                let connected = (portsc & PORTSC_CCS) != 0;
                let speed = (portsc >> 20) & 0xF;

                serial::write_str(b"xhci: port ");
                crate::serial_put_u64(port as u64);
                serial::write_str(b": connected=");
                crate::serial_put_u64(connected as u64);
                serial::write_str(b" speed=");
                crate::serial_put_u64(speed as u64);
                serial::write_str(b"\n");

                if connected {
                    // 复位端口
                    mmio_write32(mmio, psc_addr, portsc | PORTSC_PR);

                    for _ in 0..100000 {
                        let psc = mmio_read32(mmio, psc_addr);
                        if psc & PORTSC_PR == 0 {
                            break;
                        }
                        core::arch::asm!("pause");
                    }

                    serial::write_str(b"xhci: port ");
                    crate::serial_put_u64(port as u64);
                    serial::write_str(b" reset complete\n");

                    // 分配设备插槽 (简化: 直接分配 slot 1)
                    let slot = 1;
                    SLOT_ALLOC[ci] |= 1u64 << slot;

                    let psc_after = mmio_read32(mmio, psc_addr);
                    let speed_after = (psc_after >> 20) & 0xF;
                    DEVICE_SPEED[slot as usize] = speed_after as u8;
                }
            }
        }
    }
}

/// 向设备发出控制传输 (需要完整的 TRB 链实现)

pub unsafe fn control_transfer(
    _ci: usize, _slot: u8, _setup: &[u8; 8], _data: &mut [u8], _dir_in: bool,
) -> bool {
    serial::write_str(b"xhci: control transfer (stub)\n");
    false
}
