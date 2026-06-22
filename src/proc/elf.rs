// proc/elf.rs — ELF64 可执行文件解析与加载
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const ET_REL: u16 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;

/// ELF64 文件头
#[repr(C)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 程序头
#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// 已加载 ELF 镜像
pub struct LoadedElf {
    pub base: u64,
    pub entry: u64,
    pub size: u64,
}

/// 校验 ELF 头
pub fn validate(data: &[u8]) -> bool {
    if data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return false;
    }
    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Ehdr) };
    hdr.e_ident[..4] == ELF_MAGIC
        && hdr.e_ident[4] == ELFCLASS64
        && hdr.e_machine == EM_X86_64
        && (hdr.e_type == ET_REL || hdr.e_type == ET_EXEC || hdr.e_type == ET_DYN)
}

/// 将 ELF PT_LOAD 段加载到指定基址
pub unsafe fn load_segments(data: &[u8], load_base: u64) -> Option<LoadedElf> {
    if !validate(data) {
        return None;
    }
    let hdr = &*(data.as_ptr() as *const Elf64Ehdr);
    let phoff = hdr.e_phoff as usize;
    let phentsize = hdr.e_phentsize as usize;
    let phnum = hdr.e_phnum as usize;
    let mut max_end = load_base;

    for i in 0..phnum {
        let off = phoff + i * phentsize;
        if off + core::mem::size_of::<Elf64Phdr>() > data.len() {
            continue;
        }
        let ph = &*(data.as_ptr().add(off) as *const Elf64Phdr);
        if ph.p_type != PT_LOAD {
            continue;
        }
        let dst = (load_base + ph.p_vaddr) as *mut u8;
        let src = data.as_ptr().add(ph.p_offset as usize);
        if ph.p_offset as usize + ph.p_filesz as usize > data.len() {
            return None;
        }
        core::ptr::copy_nonoverlapping(src, dst, ph.p_filesz as usize);
        if ph.p_memsz > ph.p_filesz {
            let zero_start = dst.add(ph.p_filesz as usize);
            core::ptr::write_bytes(zero_start, 0, (ph.p_memsz - ph.p_filesz) as usize);
        }
        let end = load_base + ph.p_vaddr + ph.p_memsz;
        if end > max_end {
            max_end = end;
        }
    }

    Some(LoadedElf {
        base: load_base,
        entry: load_base + hdr.e_entry,
        size: max_end - load_base,
    })
}

pub fn init() {}
pub fn probe() {}
