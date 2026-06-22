// fbdev.rs — 帧缓冲设备兼容层 (仅当 DRM/KMS 不可用时启用)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::kms;

static mut FBDEV_ACTIVE: bool = false;

/// 探测 fbdev 后备路径
pub fn probe() {
    if kms::is_active() {
        crate::serial::write_str(b"drm/fbdev: skipped (KMS primary active)\n");
        return;
    }

    unsafe {
        FBDEV_ACTIVE = true;
    }
    crate::serial::write_str(b"drm/fbdev: legacy framebuffer fallback active\n");
}

/// fbdev 是否作为当前显示后端
pub fn is_active() -> bool {
    unsafe { FBDEV_ACTIVE }
}
