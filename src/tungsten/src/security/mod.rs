// security/mod.rs — 安全子系统 (LSM/ACL/Capabilities/Audit)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod lsm; pub mod acl; pub mod caps; pub mod audit; pub mod tpm; pub mod keyring;
pub fn init() { lsm::init(); acl::init(); caps::init(); audit::init(); tpm::init();
  crate::serial::write_str(b"security: subsystem ready\n"); }
