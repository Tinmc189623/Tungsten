// kvm/mod.rs — 内核虚拟机 (KVM) — 硬件虚拟化 (VMX/SVM)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

pub mod vmx; pub mod svm; pub mod vcpu; pub mod mmu;
use crate::sync::SpinLock;
pub struct KvmManager { pub vmx_supported: bool, pub svm_supported: bool, pub vm_count: u32 }
static KVM_MGR: SpinLock<KvmManager> = SpinLock::new(KvmManager { vmx_supported: false, svm_supported: false, vm_count: 0 });
pub fn init() { vmx::probe(); svm::probe(); crate::serial::write_str(b"kvm: ready\n"); }
pub fn sys_kvm_create_vm() -> i32 { -1 }
pub fn sys_kvm_create_vcpu(_vm_fd: i32, _vcpu_id: u32) -> i32 { -1 }
