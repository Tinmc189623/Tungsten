// block/virtio_blk.rs — VirtIO 块设备驱动 (PCI 1.0 + Virtqueue)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::common;
use super::{register_device, BlockDevice, BlockOps};
use crate::devices::pci;
use crate::mm::pmm;
use crate::mm::vmm;
use crate::sync::SpinLock;

const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_DEV_BLK: u16 = 0x1042;
const VIRTIO_DEV_BLK_LEGACY: u16 = 0x1001;

const VIRTIO_STATUS_ACK: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;

const MMIO_QUEUE_SEL: u64 = 0x030;
const MMIO_QUEUE_NUM_MAX: u64 = 0x034;
const MMIO_QUEUE_NUM: u64 = 0x038;
const MMIO_QUEUE_READY: u64 = 0x044;
const MMIO_QUEUE_NOTIFY: u64 = 0x050;
const MMIO_INTERRUPT_STATUS: u64 = 0x060;
const MMIO_INTERRUPT_ACK: u64 = 0x064;
const MMIO_STATUS: u64 = 0x070;
const MMIO_QUEUE_DESC_LOW: u64 = 0x080;
const MMIO_QUEUE_DESC_HIGH: u64 = 0x084;
const MMIO_QUEUE_DRIVER_LOW: u64 = 0x090;
const MMIO_QUEUE_DRIVER_HIGH: u64 = 0x094;
const MMIO_QUEUE_DEVICE_LOW: u64 = 0x0a0;
const MMIO_QUEUE_DEVICE_HIGH: u64 = 0x0a4;
const MMIO_CONFIG: u64 = 0x100;

const QUEUE_SIZE: u16 = 16;
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

#[repr(C)]
struct VirtioBlkReq {
    typ: u32,
    reserved: u32,
    sector: u64,
}

#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE as usize],
}

#[repr(C)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    ring: [VirtqUsedElem; QUEUE_SIZE as usize],
}

/// Virtqueue 内存布局（单页对齐）
struct VirtqueueMem {
    phys: u64,
    virt: u64,
}

/// VirtIO 块设备运行时状态
struct VirtioBlkState {
    mmio: u64,
    queue: VirtqueueMem,
    capacity: u64,
    free_head: u16,
    avail_idx: u16,
    last_used_idx: u16,
}

static VIRTIO_BLK: SpinLock<Option<VirtioBlkState>> = SpinLock::new(None);

/// 向 MMIO 寄存器写入 64 位物理地址（低/高 32 位分拆）
#[inline]
unsafe fn mmio_write_phys64(mmio: u64, reg_low: u64, phys: u64) {
    common::mmio_write32(mmio + reg_low, phys as u32);
    common::mmio_write32(mmio + reg_low + 4, (phys >> 32) as u32);
}

/// 初始化 Virtqueue 并返回设备容量（扇区数）
unsafe fn setup_virtio(mmio: u64) -> Option<u64> {
    let magic = common::mmio_read32(mmio);
    if magic != 0x74726976 {
        return None;
    }
    common::mmio_write32(mmio + MMIO_STATUS, 0);
    common::mmio_write32(mmio + MMIO_STATUS, VIRTIO_STATUS_ACK);
    common::mmio_write32(mmio + MMIO_STATUS, VIRTIO_STATUS_ACK | VIRTIO_STATUS_DRIVER);

    common::mmio_write32(mmio + MMIO_QUEUE_SEL, 0);
    let num_max = common::mmio_read32(mmio + MMIO_QUEUE_NUM_MAX);
    let qnum = num_max.min(QUEUE_SIZE as u32) as u16;
    if qnum == 0 {
        return None;
    }

    let queue_phys = pmm::alloc_zeroed()?;
    let queue_virt = vmm::phys_to_virt(queue_phys);

    common::mmio_write32(mmio + MMIO_QUEUE_NUM, qnum as u32);
    mmio_write_phys64(mmio, MMIO_QUEUE_DESC_LOW, queue_phys);
    let avail_off = (core::mem::size_of::<VirtqDesc>() * qnum as usize) as u64;
    let avail_phys = queue_phys + avail_off;
    mmio_write_phys64(mmio, MMIO_QUEUE_DRIVER_LOW, avail_phys);
    let used_off = avail_off + core::mem::size_of::<VirtqAvail>() as u64;
    let used_phys = queue_phys + used_off;
    mmio_write_phys64(mmio, MMIO_QUEUE_DEVICE_LOW, used_phys);
    common::mmio_write32(mmio + MMIO_QUEUE_READY, 1);
    common::mmio_write32(
        mmio + MMIO_STATUS,
        VIRTIO_STATUS_ACK | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK,
    );

    let cap_low = common::mmio_read32(mmio + MMIO_CONFIG) as u64;
    let cap_high = common::mmio_read32(mmio + MMIO_CONFIG + 4) as u64;
    let capacity = cap_low | (cap_high << 32);

    let mut guard = VIRTIO_BLK.lock();
    *guard = Some(VirtioBlkState {
        mmio,
        queue: VirtqueueMem {
            phys: queue_phys,
            virt: queue_virt,
        },
        capacity,
        free_head: 0,
        avail_idx: 0,
        last_used_idx: 0,
    });
    Some(capacity)
}

/// 提交 VirtIO 块请求并等待完成
unsafe fn submit_blk_io(typ: u32, sector: u64, data: *mut u8, data_len: usize) -> i32 {
    let mut guard = VIRTIO_BLK.lock();
    let Some(state) = guard.as_mut() else {
        return -19;
    };
    let mmio = state.mmio;
    let qvirt = state.queue.virt;
    let desc = qvirt as *mut VirtqDesc;
    let avail = (qvirt + (core::mem::size_of::<VirtqDesc>() * QUEUE_SIZE as usize) as u64)
        as *mut VirtqAvail;
    let used = (qvirt
        + (core::mem::size_of::<VirtqDesc>() * QUEUE_SIZE as usize) as u64
        + core::mem::size_of::<VirtqAvail>() as u64) as *mut VirtqUsed;

    let hdr_phys = match pmm::alloc_zeroed() {
        Some(p) => p,
        None => return -12,
    };
    let req = vmm::phys_to_virt(hdr_phys) as *mut VirtioBlkReq;
    (*req).typ = typ;
    (*req).reserved = 0;
    (*req).sector = sector;

    let status_phys = match pmm::alloc_zeroed() {
        Some(p) => p,
        None => return -12,
    };
    core::ptr::write(vmm::phys_to_virt(status_phys) as *mut u8, 0xFF);

    let data_phys = if data_len > 0 {
        match pmm::alloc_zeroed() {
            Some(p) => p,
            None => return -12,
        }
    } else {
        0
    };
    if typ == VIRTIO_BLK_T_OUT && data_len > 0 {
        core::ptr::copy_nonoverlapping(data, vmm::phys_to_virt(data_phys) as *mut u8, data_len);
    }

    let head = state.free_head;
    (*desc.add(0)).addr = hdr_phys;
    (*desc.add(0)).len = core::mem::size_of::<VirtioBlkReq>() as u32;
    (*desc.add(0)).flags = DESC_F_NEXT;
    (*desc.add(0)).next = 1;

    if data_len > 0 {
        (*desc.add(1)).addr = data_phys;
        (*desc.add(1)).len = data_len as u32;
        (*desc.add(1)).flags = DESC_F_NEXT | if typ == VIRTIO_BLK_T_IN { DESC_F_WRITE } else { 0 };
        (*desc.add(1)).next = 2;
    }

    let status_desc = if data_len > 0 { 2 } else { 1 };
    (*desc.add(status_desc)).addr = status_phys;
    (*desc.add(status_desc)).len = 1;
    (*desc.add(status_desc)).flags = DESC_F_WRITE;
    (*desc.add(status_desc)).next = 0;

    let slot = (state.avail_idx % QUEUE_SIZE) as usize;
    (*avail).ring[slot] = head;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    state.avail_idx = state.avail_idx.wrapping_add(1);
    (*avail).idx = state.avail_idx;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

    common::mmio_write32(mmio + MMIO_QUEUE_NOTIFY, 0);

    for _ in 0..1_000_000 {
        let used_idx = (*used).idx;
        if used_idx != state.last_used_idx {
            state.last_used_idx = used_idx;
            let status = core::ptr::read(vmm::phys_to_virt(status_phys) as *const u8);
            if status != 0 {
                return -5;
            }
            if typ == VIRTIO_BLK_T_IN && data_len > 0 {
                core::ptr::copy_nonoverlapping(
                    vmm::phys_to_virt(data_phys) as *const u8,
                    data,
                    data_len,
                );
            }
            let _ = common::mmio_read32(mmio + MMIO_INTERRUPT_STATUS);
            common::mmio_write32(mmio + MMIO_INTERRUPT_ACK, 1);
            return data_len as i32;
        }
    }
    -110
}

unsafe extern "C" fn virtio_read(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    buf: *mut u8,
) -> i32 {
    let d = unsafe { &*dev };
    if count == 0 {
        return 0;
    }
    let byte_len = (count as u64).saturating_mul(512) as usize;
    if byte_len > 65536 {
        return -22;
    }
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    submit_blk_io(VIRTIO_BLK_T_IN, lba, buf, byte_len)
}

unsafe extern "C" fn virtio_write(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    buf: *const u8,
) -> i32 {
    let d = unsafe { &*dev };
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    let byte_len = (count as u64).saturating_mul(512) as usize;
    submit_blk_io(VIRTIO_BLK_T_OUT, lba, buf as *mut u8, byte_len)
}

unsafe extern "C" fn virtio_flush(_dev: *mut BlockDevice) -> i32 {
    let mut status_buf = [0u8; 1];
    submit_blk_io(VIRTIO_BLK_T_FLUSH, 0, status_buf.as_mut_ptr(), 0)
}

unsafe extern "C" fn virtio_trim(_dev: *mut BlockDevice, _lba: u64, _count: u32) -> i32 {
    0
}

static VIRTIO_OPS: BlockOps = BlockOps {
    read: virtio_read,
    write: virtio_write,
    flush: virtio_flush,
    trim: virtio_trim,
};

/// 探测 PCI VirtIO 块设备
pub fn probe() {
    crate::devices::pci::enumerate_all();
    for d in pci::devices() {
        if d.vendor_id != VIRTIO_VENDOR {
            continue;
        }
        if d.device_id != VIRTIO_DEV_BLK && d.device_id != VIRTIO_DEV_BLK_LEGACY {
            continue;
        }
        let bar = d.bars[0];
        let mmio = common::map_pci_bar(bar);
        if mmio == 0 {
            continue;
        }
        let capacity = unsafe { setup_virtio(mmio).unwrap_or(0) };
        if capacity == 0 {
            continue;
        }
        let mut name = [0u8; 32];
        common::copy_name(&mut name, b"virtio0");
        register_device(BlockDevice {
            name,
            major: 252,
            minor: 0,
            sector_size: 512,
            total_sectors: capacity,
            max_transfer: 128,
            flags: 0,
            ops: &VIRTIO_OPS,
            priv_data: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        });
        crate::serial::write_str(b"  virtio-blk: virtqueue online, capacity=");
        crate::serial_put_u64(capacity);
        crate::serial::write_str(b" sectors\n");
        return;
    }
    crate::serial::write_str(b"  virtio-blk: not found\n");
}

pub fn init() {}
