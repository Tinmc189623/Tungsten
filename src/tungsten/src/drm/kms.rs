// kms.rs — Kernel Mode Setting (Limine 线性帧缓冲 DRM 后端)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{DrmCaps, DrmDevice, DrmDeviceOps, DrmFramebuffer, register_primary_device};
use crate::limine_boot;

static mut LIMINE_FB_ADDR: u64 = 0;
static mut LIMINE_FB_WIDTH: u32 = 0;
static mut LIMINE_FB_HEIGHT: u32 = 0;
static mut LIMINE_FB_PITCH: u32 = 0;
static mut LIMINE_FB_BPP: u32 = 32;
static mut KMS_READY: bool = false;

static LIMINE_OPS: DrmDeviceOps = DrmDeviceOps {
    modeset: limine_modeset,
    page_flip: limine_page_flip,
    create_fb: limine_create_fb,
    destroy_fb: limine_destroy_fb,
    map_fb: limine_map_fb,
    set_cursor: limine_set_cursor,
    vsync_wait: limine_vsync_wait,
};

static mut LIMINE_DEVICE: DrmDevice = DrmDevice {
    name: [0u8; 32],
    vendor: [0u8; 32],
    caps: DrmCaps {
        max_width: 0,
        max_height: 0,
        cursor_width: 64,
        cursor_height: 64,
        supports_prime: false,
        supports_dmabuf: false,
        supports_atomic: false,
        supports_modeset: true,
    },
    fb: DrmFramebuffer {
        width: 0,
        height: 0,
        pitch: 0,
        bpp: 32,
        format: 0x34325258,
        size: 0,
        handle: 1,
    },
    ops: &LIMINE_OPS,
    priv_data: core::ptr::null_mut(),
};

/// 填充设备名称字段
fn write_name_field(dst: &mut [u8; 32], src: &[u8]) {
    let len = core::cmp::min(31, src.len());
    dst[..len].copy_from_slice(&src[..len]);
    dst[len] = 0;
}

/// 探测并注册 Limine KMS 设备
pub fn probe() {
    let Some(bi) = limine_boot::cached_boot_info() else {
        crate::serial::write_str(b"drm/kms: no boot framebuffer\n");
        return;
    };

    if bi.fb_addr == 0 || bi.fb_width == 0 || bi.fb_height == 0 {
        crate::serial::write_str(b"drm/kms: invalid framebuffer params\n");
        return;
    }

    unsafe {
        let dev = core::ptr::addr_of_mut!(LIMINE_DEVICE);
        write_name_field(&mut (*dev).name, b"limine-linear-fb");
        write_name_field(&mut (*dev).vendor, b"Nexsteaduser");

        LIMINE_FB_ADDR = bi.fb_addr;
        LIMINE_FB_WIDTH = bi.fb_width as u32;
        LIMINE_FB_HEIGHT = bi.fb_height as u32;
        LIMINE_FB_PITCH = bi.fb_pitch as u32;
        LIMINE_FB_BPP = if bi.fb_bpp == 0 { 32 } else { bi.fb_bpp };

        let size = (bi.fb_pitch as u64).saturating_mul(bi.fb_height);

        (*dev).caps.max_width = LIMINE_FB_WIDTH;
        (*dev).caps.max_height = LIMINE_FB_HEIGHT;
        (*dev).fb.width = LIMINE_FB_WIDTH;
        (*dev).fb.height = LIMINE_FB_HEIGHT;
        (*dev).fb.pitch = LIMINE_FB_PITCH;
        (*dev).fb.bpp = LIMINE_FB_BPP;
        (*dev).fb.size = size;

        register_primary_device(dev);
        KMS_READY = true;
    }

    crate::serial::write_str(b"drm/kms: Limine linear framebuffer registered (primary)\n");
}

/// KMS 是否已接管主显示
pub fn is_active() -> bool {
    unsafe { KMS_READY }
}

/// KMS 扫描输出到帧缓冲
pub fn scanout() {
    if !is_active() {
        return;
    }
    core::hint::spin_loop();
}

unsafe extern "C" fn limine_modeset(
    dev: *mut DrmDevice, w: u32, h: u32, bpp: u32,
) -> i32 {
    if dev.is_null() {
        return -1;
    }
    (*dev).fb.width = w;
    (*dev).fb.height = h;
    (*dev).fb.bpp = bpp;
    0
}

unsafe extern "C" fn limine_page_flip(_dev: *mut DrmDevice, _fb_id: u32) -> i32 {
    0
}

unsafe extern "C" fn limine_create_fb(
    dev: *mut DrmDevice, w: u32, h: u32, fmt: u32,
) -> u32 {
    if dev.is_null() {
        return 0;
    }
    (*dev).fb.width = w;
    (*dev).fb.height = h;
    (*dev).fb.format = fmt;
    1
}

unsafe extern "C" fn limine_destroy_fb(_dev: *mut DrmDevice, _fb_id: u32) {}

unsafe extern "C" fn limine_map_fb(_dev: *mut DrmDevice, _fb_id: u32) -> *mut u8 {
    LIMINE_FB_ADDR as *mut u8
}

unsafe extern "C" fn limine_set_cursor(_dev: *mut DrmDevice, _x: i32, _y: i32) {}

unsafe extern "C" fn limine_vsync_wait(_dev: *mut DrmDevice) {
    let _ = _dev;
    core::hint::spin_loop();
}
