// drm/mod.rs — Direct Rendering Manager 框架 (KMS/DRM)
// 规则 44: 系统默认优先使用 DRM
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod kms;
pub mod gem;
pub mod fbdev;
pub mod vesa;

use crate::sync::SpinLock;

#[repr(C)]
pub struct DrmDevice {
    pub name: [u8; 32],
    pub vendor: [u8; 32],
    pub caps: DrmCaps,
    pub fb: DrmFramebuffer,
    pub ops: &'static DrmDeviceOps,
    pub priv_data: *mut (),
}

#[repr(C)]
pub struct DrmCaps {
    pub max_width: u32,
    pub max_height: u32,
    pub cursor_width: u32,
    pub cursor_height: u32,
    pub supports_prime: bool,
    pub supports_dmabuf: bool,
    pub supports_atomic: bool,
    pub supports_modeset: bool,
}

#[repr(C)]
pub struct DrmFramebuffer {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub format: u32,
    pub size: u64,
    pub handle: u32,
}

#[repr(C)]
pub struct DrmDeviceOps {
    pub modeset: unsafe extern "C" fn(dev: *mut DrmDevice, w: u32, h: u32, bpp: u32) -> i32,
    pub page_flip: unsafe extern "C" fn(dev: *mut DrmDevice, fb_id: u32) -> i32,
    pub create_fb: unsafe extern "C" fn(dev: *mut DrmDevice, w: u32, h: u32, fmt: u32) -> u32,
    pub destroy_fb: unsafe extern "C" fn(dev: *mut DrmDevice, fb_id: u32),
    pub map_fb: unsafe extern "C" fn(dev: *mut DrmDevice, fb_id: u32) -> *mut u8,
    pub set_cursor: unsafe extern "C" fn(dev: *mut DrmDevice, x: i32, y: i32),
    pub vsync_wait: unsafe extern "C" fn(dev: *mut DrmDevice),
}

pub struct DrmManager {
    pub primary: *mut DrmDevice,
    pub devices: *mut DrmDevice,
    pub count: usize,
}

unsafe impl Send for DrmManager {}

static DRM_MGR: SpinLock<DrmManager> = SpinLock::new(DrmManager {
    primary: core::ptr::null_mut(),
    devices: core::ptr::null_mut(),
    count: 0,
});

/// 注册主 DRM 设备 (由 KMS 后端调用)
pub fn register_primary_device(dev: *mut DrmDevice) {
    let mut mgr = DRM_MGR.lock();
    mgr.primary = dev;
    mgr.devices = dev;
    mgr.count = 1;
}

pub fn init() {
    crate::serial::write_str(b"drm: initializing DRM subsystem...\n");
    crate::serial::write_str(b"drm: probing KMS (primary)...\n");
    kms::probe();
    if kms::is_active() {
        crate::serial::write_str(b"drm: KMS primary display ready\n");
    } else {
        crate::serial::write_str(b"drm: KMS unavailable, enabling fbdev fallback\n");
    }
    crate::serial::write_str(b"drm: probing GEM...\n");
    gem::probe();
    fbdev::probe();
    if !kms::is_active() && !fbdev::is_active() {
        crate::serial::write_str(b"drm: WARN no display backend\n");
    }
    crate::serial::write_str(b"drm: subsystem ready\n");
}

/// 显示服务刷新（由 displayd 调用）
pub fn refresh() {
    kms::scanout();
}
