// cpu.rs — x86_64 CPU 特性检测 (CPUID)
// 检测所有 CPUID 叶, 缓存结果供内核和驱动使用
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

impl CpuidResult {
    pub const fn new() -> Self {
        CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 }
    }
}

#[repr(C)]
pub struct CpuFeatures {
    pub fpu: bool,
    pub vme: bool,
    pub de: bool,
    pub pse: bool,
    pub tsc: bool,
    pub msr: bool,
    pub pae: bool,
    pub mce: bool,
    pub cx8: bool,
    pub apic: bool,
    pub sep: bool,
    pub mtrr: bool,
    pub pge: bool,
    pub mca: bool,
    pub cmov: bool,
    pub pat: bool,
    pub pse36: bool,
    pub psn: bool,
    pub clfsh: bool,
    pub ds: bool,
    pub acpi: bool,
    pub mmx: bool,
    pub fxsr: bool,
    pub sse: bool,
    pub sse2: bool,
    pub ss: bool,
    pub htt: bool,
    pub tm: bool,
    pub pbe: bool,
    pub sse3: bool,
    pub pclmulqdq: bool,
    pub dtes64: bool,
    pub monitor: bool,
    pub dscpl: bool,
    pub vmx: bool,
    pub smx: bool,
    pub est: bool,
    pub tm2: bool,
    pub ssse3: bool,
    pub cid: bool,
    pub sdbg: bool,
    pub fma: bool,
    pub cx16: bool,
    pub xtpr: bool,
    pub pdcm: bool,
    pub pcid: bool,
    pub dca: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub x2apic: bool,
    pub movbe: bool,
    pub popcnt: bool,
    pub tsc_deadline: bool,
    pub aesni: bool,
    pub xsave: bool,
    pub osxsave: bool,
    pub avx: bool,
    pub f16c: bool,
    pub rdrand: bool,
    pub fsgsbase: bool,
    pub bmi1: bool,
    pub hle: bool,
    pub avx2: bool,
    pub smep: bool,
    pub bmi2: bool,
    pub erms: bool,
    pub invpcid: bool,
    pub rtm: bool,
    pub pqm: bool,
    pub mpx: bool,
    pub avx512f: bool,
    pub avx512dq: bool,
    pub rdseed: bool,
    pub adx: bool,
    pub smap: bool,
    pub avx512ifma: bool,
    pub clflushopt: bool,
    pub clwb: bool,
    pub sha_ni: bool,
    pub avx512bw: bool,
    pub avx512vl: bool,
    pub avx512vbmi: bool,
    pub umip: bool,
    pub pku: bool,
    pub ospke: bool,
    pub avx512_vnni: bool,
    pub avx512_bitalg: bool,
    pub avx512_vpopcntdq: bool,
    pub rdpid: bool,
    pub cldemote: bool,
    pub movdiri: bool,
    pub movdir64b: bool,
    pub syscall: bool,
    pub nx: bool,
    pub page1gb: bool,
    pub rdtscp: bool,
    pub lm: bool,
    pub lahf_sahf: bool,
    pub svm: bool,
    pub sse4a: bool,
    pub xop: bool,
    pub fma4: bool,
    pub tbm: bool,
    pub vendor: [u8; 13],
    pub brand: [u8; 49],
    pub max_basic_leaf: u32,
    pub max_ext_leaf: u32,
    pub family: u8,
    pub model: u8,
    pub stepping: u8,
    pub cores: u8,
    pub threads: u8,
}

impl CpuFeatures {
    pub const fn new() -> Self {
        CpuFeatures {
            fpu: false, vme: false, de: false, pse: false, tsc: false, msr: false, pae: false,
            mce: false, cx8: false, apic: false, sep: false, mtrr: false, pge: false, mca: false,
            cmov: false, pat: false, pse36: false, psn: false, clfsh: false, ds: false, acpi: false,
            mmx: false, fxsr: false, sse: false, sse2: false, ss: false, htt: false, tm: false, pbe: false,
            sse3: false, pclmulqdq: false, dtes64: false, monitor: false, dscpl: false, vmx: false,
            smx: false, est: false, tm2: false, ssse3: false, cid: false, sdbg: false, fma: false,
            cx16: false, xtpr: false, pdcm: false, pcid: false, dca: false, sse4_1: false, sse4_2: false,
            x2apic: false, movbe: false, popcnt: false, tsc_deadline: false, aesni: false, xsave: false,
            osxsave: false, avx: false, f16c: false, rdrand: false, fsgsbase: false, bmi1: false,
            hle: false, avx2: false, smep: false, bmi2: false, erms: false, invpcid: false, rtm: false,
            pqm: false, mpx: false, avx512f: false, avx512dq: false, rdseed: false, adx: false, smap: false,
            avx512ifma: false, clflushopt: false, clwb: false, sha_ni: false, avx512bw: false, avx512vl: false,
            avx512vbmi: false, umip: false, pku: false, ospke: false, avx512_vnni: false, avx512_bitalg: false,
            avx512_vpopcntdq: false, rdpid: false, cldemote: false, movdiri: false, movdir64b: false,
            syscall: false, nx: false, page1gb: false, rdtscp: false, lm: false, lahf_sahf: false, svm: false,
            sse4a: false, xop: false, fma4: false, tbm: false,
            vendor: [0; 13], brand: [0; 49], max_basic_leaf: 0, max_ext_leaf: 0,
            family: 0, model: 0, stepping: 0, cores: 1, threads: 1,
        }
    }
}

static mut CPU_FEATURES: CpuFeatures = CpuFeatures::new();
static CPU_READY: AtomicBool = AtomicBool::new(false);

/// 读取 CPUID 叶
unsafe fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    let mut eax: u32 = leaf;
    let mut ecx: u32 = subleaf;
    let mut edx: u32 = 0;
    let ebx: u32;
    asm!(
        "push rbx; cpuid; mov {0:e}, ebx; pop rbx",
        out(reg) ebx,
        inout("eax") eax,
        inout("ecx") ecx,
        inout("edx") edx,
    );
    CpuidResult { eax, ebx, ecx, edx }
}

/// 返回厂商字符串切片（detect 之后只读）
fn vendor_slice() -> &'static [u8] {
    let vendor = unsafe { &*core::ptr::addr_of!(CPU_FEATURES.vendor) };
    let end = vendor.iter().position(|&b| b == 0).unwrap_or(12);
    &vendor[..end]
}

/// 执行 CPUID 检测并缓存结果
pub fn detect() {
    unsafe {
        let r = cpuid(0, 0);
        CPU_FEATURES.max_basic_leaf = r.eax;
        CPU_FEATURES.vendor[0] = r.ebx as u8;
        CPU_FEATURES.vendor[1] = (r.ebx >> 8) as u8;
        CPU_FEATURES.vendor[2] = (r.ebx >> 16) as u8;
        CPU_FEATURES.vendor[3] = (r.ebx >> 24) as u8;
        CPU_FEATURES.vendor[4] = r.edx as u8;
        CPU_FEATURES.vendor[5] = (r.edx >> 8) as u8;
        CPU_FEATURES.vendor[6] = (r.edx >> 16) as u8;
        CPU_FEATURES.vendor[7] = (r.edx >> 24) as u8;
        CPU_FEATURES.vendor[8] = r.ecx as u8;
        CPU_FEATURES.vendor[9] = (r.ecx >> 8) as u8;
        CPU_FEATURES.vendor[10] = (r.ecx >> 16) as u8;
        CPU_FEATURES.vendor[11] = (r.ecx >> 24) as u8;

        if r.eax >= 1 {
            let r1 = cpuid(1, 0);
            CPU_FEATURES.fpu = r1.edx & 1 != 0;
            CPU_FEATURES.tsc = r1.edx & (1 << 4) != 0;
            CPU_FEATURES.msr = r1.edx & (1 << 5) != 0;
            CPU_FEATURES.apic = r1.edx & (1 << 9) != 0;
            CPU_FEATURES.mmx = r1.edx & (1 << 23) != 0;
            CPU_FEATURES.fxsr = r1.edx & (1 << 24) != 0;
            CPU_FEATURES.sse = r1.edx & (1 << 25) != 0;
            CPU_FEATURES.sse2 = r1.edx & (1 << 26) != 0;
            CPU_FEATURES.htt = r1.edx & (1 << 28) != 0;
            CPU_FEATURES.sse3 = r1.ecx & 1 != 0;
            CPU_FEATURES.vmx = r1.ecx & (1 << 5) != 0;
            CPU_FEATURES.ssse3 = r1.ecx & (1 << 9) != 0;
            CPU_FEATURES.fma = r1.ecx & (1 << 12) != 0;
            CPU_FEATURES.sse4_1 = r1.ecx & (1 << 19) != 0;
            CPU_FEATURES.sse4_2 = r1.ecx & (1 << 20) != 0;
            CPU_FEATURES.x2apic = r1.ecx & (1 << 21) != 0;
            CPU_FEATURES.aesni = r1.ecx & (1 << 25) != 0;
            CPU_FEATURES.avx = r1.ecx & (1 << 28) != 0;
            CPU_FEATURES.rdrand = r1.ecx & (1 << 30) != 0;
            CPU_FEATURES.family = ((r1.eax >> 8) & 0xF) as u8;
            CPU_FEATURES.model = ((r1.eax >> 4) & 0xF) as u8;
            CPU_FEATURES.stepping = (r1.eax & 0xF) as u8;
        }

        if CPU_FEATURES.max_basic_leaf >= 7 {
            let r7 = cpuid(7, 0);
            CPU_FEATURES.fsgsbase = r7.ebx & 1 != 0;
            CPU_FEATURES.bmi1 = r7.ebx & (1 << 3) != 0;
            CPU_FEATURES.avx2 = r7.ebx & (1 << 5) != 0;
            CPU_FEATURES.smep = r7.ebx & (1 << 7) != 0;
            CPU_FEATURES.bmi2 = r7.ebx & (1 << 8) != 0;
            CPU_FEATURES.rdseed = r7.ebx & (1 << 18) != 0;
            CPU_FEATURES.adx = r7.ebx & (1 << 19) != 0;
            CPU_FEATURES.smap = r7.ebx & (1 << 20) != 0;
            CPU_FEATURES.sha_ni = r7.ebx & (1 << 29) != 0;
        }

        let re = cpuid(0x8000_0000, 0);
        CPU_FEATURES.max_ext_leaf = re.eax;
        if re.eax >= 0x8000_0001 {
            let r81 = cpuid(0x8000_0001, 0);
            CPU_FEATURES.syscall = r81.edx & (1 << 11) != 0;
            CPU_FEATURES.nx = r81.edx & (1 << 20) != 0;
            CPU_FEATURES.page1gb = r81.edx & (1 << 26) != 0;
            CPU_FEATURES.rdtscp = r81.edx & (1 << 27) != 0;
            CPU_FEATURES.lm = r81.edx & (1 << 29) != 0;
            CPU_FEATURES.lahf_sahf = r81.ecx & 1 != 0;
            CPU_FEATURES.svm = r81.ecx & (1 << 2) != 0;
        }

        crate::serial::write_str(b"cpu: ");
        crate::serial::write_str(vendor_slice());
        crate::serial::write_str(b" family=");
        crate::serial_put_u64(CPU_FEATURES.family as u64);
        crate::serial::write_str(b" model=");
        crate::serial_put_u64(CPU_FEATURES.model as u64);
        crate::serial::write_str(b" sse=");
        if CPU_FEATURES.sse {
            crate::serial::write_str(b"yes");
        } else {
            crate::serial::write_str(b"no");
        }
        crate::serial::write_str(b" avx=");
        if CPU_FEATURES.avx {
            crate::serial::write_str(b"yes");
        } else {
            crate::serial::write_str(b"no");
        }
        crate::serial::write_str(b"\n");
    }
    CPU_READY.store(true, Ordering::Release);
}

/// 获取缓存的 CPU 特性（detect 之后有效）
pub fn features() -> &'static CpuFeatures {
    debug_assert!(CPU_READY.load(Ordering::Acquire));
    unsafe { &*core::ptr::addr_of!(CPU_FEATURES) }
}

/// 是否支持 SSE
pub fn has_sse() -> bool {
    CPU_READY.load(Ordering::Acquire) && unsafe { CPU_FEATURES.sse }
}

/// 是否支持 AVX
pub fn has_avx() -> bool {
    CPU_READY.load(Ordering::Acquire) && unsafe { CPU_FEATURES.avx }
}

/// 是否支持 AES-NI
pub fn has_aesni() -> bool {
    CPU_READY.load(Ordering::Acquire) && unsafe { CPU_FEATURES.aesni }
}

/// 返回 CPU 厂商字符串
pub fn vendor_str() -> &'static [u8] {
    vendor_slice()
}
