// usb/mod.rs — USB 子系统 (XHCI/EHCI/OHCI)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod xhci; pub mod hid;
use crate::sync::SpinLock;
pub struct UsbManager { pub initialized: bool }
static USB_MGR: SpinLock<UsbManager> = SpinLock::new(UsbManager { initialized: false });
pub fn init() { crate::serial::write_str(b"usb: initializing...\n"); xhci::probe(); hid::init();
  USB_MGR.lock().initialized = true; crate::serial::write_str(b"usb: ready\n"); }
