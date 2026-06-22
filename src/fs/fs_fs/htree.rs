// fs/fs_fs/htree.rs — HTree 目录哈希索引 (ext4 half-MD4 同款)
// 将目录项按名称哈希分布到 B+tree 索引, O(log N) 查找
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::fs_fs::extent::ExtentTree;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── HTree 常量 ──

/// HTree 根节点魔数
const HTREE_MAGIC: u32 = 0x4854_5245; // "HTRE"

/// HTree 节点大小 (每个 hash 块对齐到 4096)
const HTREE_BLOCK_SIZE: u64 = 4096;

/// 每个 HTree 块的最大条目数
const HTREE_ENTRIES_PER_BLOCK: u16 = ((HTREE_BLOCK_SIZE as usize - 16) / 8) as u16; // ~510

/// 小目录阈值 (目录项数小于此值用线性扫描)
const HTREE_SMALL_DIR: u16 = 16;

// ── HTree 根节点 (位于目录文件偏移 0) ──

#[repr(C, packed)]
struct HtreeRoot {
    magic: u32,            // HTREE_MAGIC
    hash_version: u8,      // 0 = half-MD4, 1 = half-MD4+seed
    info_length: u8,       // 节点头大小 (16)
    indirect_levels: u8,   // 间接层数
    _unused: u8,
    entries_count: u16,    // 当前条目数
    entries_limit: u16,    // 最大条目数
    seed: u32,             // 哈希种子 (防止哈希碰撞攻击)
    checksum: u32,
}

impl HtreeRoot {
    const fn empty() -> Self {
        HtreeRoot {
            magic: 0, hash_version: 0, info_length: 16,
            indirect_levels: 0, _unused: 0,
            entries_count: 0, entries_limit: 0,
            seed: 0, checksum: 0,
        }
    }

    fn init(&mut self, seed: u32) {
        self.magic = HTREE_MAGIC;
        self.hash_version = 0;
        self.info_length = 16;
        self.indirect_levels = 0;
        self.entries_count = 0;
        self.entries_limit = HTREE_ENTRIES_PER_BLOCK;
        self.seed = seed;
        self.checksum = 0;
    }
}

// ── HTree 条目 ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HtreeEntry {
    hash: u32,
    block_offset: u32,     // 目录文件内逻辑偏移 (4KB 对齐)
}

// ── HTree 内部节点头 ──

#[repr(C, packed)]
struct HtreeNodeHeader {
    entries_count: u16,
    entries_limit: u16,
    checksum: u32,
}

// ── half-MD4 哈希 ──

/// F, G 轮函数
fn md4_f(x: u32, y: u32, z: u32) -> u32 { (x & y) | (!x & z) }
fn md4_g(x: u32, y: u32, z: u32) -> u32 { (x & y) | (x & z) | (y & z) }

fn rotl(x: u32, n: u32) -> u32 { (x << n) | (x >> (32 - n)) }

/// half-MD4: MD4 的前两轮 (不含第三轮), 用于目录哈希
/// 输入: 以 NUL 结尾的名称, 种子
/// 输出: 32 位哈希值
pub fn half_md4(name: &[u8], seed: u32) -> u32 {
    let mut a: u32 = seed;
    let mut b: u32 = 0x6745_2301;
    let mut c: u32 = 0xEFCD_AB89;
    let mut d: u32 = 0x98BA_DCFE;

    // 填充到 64 字节块 (含 NUL 终止)
    let len = name.len();
    let total_len = len + 1; // + NUL
    let padded_len = ((total_len + 63) / 64) * 64;
    let mut buf = [0u8; 64];

    for chunk_start in (0..padded_len).step_by(64) {
        if chunk_start == 0 {
            // 第一块: 名称 + NUL + 填充 (含长度)
            let copy_len = len.min(64);
            buf[..copy_len].copy_from_slice(&name[..copy_len]);
            if copy_len < 64 {
                buf[copy_len] = 0x80; // MD4 padding byte
            }
            // 长度编码在最后 8 字节
            if padded_len == 64 {
                let bit_len = (total_len * 8) as u64;
                buf[56..64].copy_from_slice(&bit_len.to_le_bytes());
            }
        } else if chunk_start + 64 == padded_len {
            // 最后一块: 存放长度
            buf.fill(0);
            let bit_len = (total_len * 8) as u64;
            buf[56..64].copy_from_slice(&bit_len.to_le_bytes());
        } else {
            buf.fill(0);
        }

        // 加载 16 个 32 位字 (小端)
        let mut x = [0u32; 16];
        for i in 0..16 {
            let off = i * 4;
            x[i] = u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
        }

        let (aa, bb, cc, dd) = (a, b, c, d);

        // 第 1 轮
        let s1: [u32; 16] = [3, 7, 11, 19, 3, 7, 11, 19, 3, 7, 11, 19, 3, 7, 11, 19];
        for i in 0..16 {
            let k = i;
            let s = s1[i];
            let f = md4_f(b, c, d);
            a = rotl(a.wrapping_add(f).wrapping_add(x[k]), s);
            (a, b, c, d) = (d, a, b, c);
        }

        // 第 2 轮
        let idx2: [usize; 16] = [0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];
        let s2: [u32; 16] = [3, 5, 9, 13, 3, 5, 9, 13, 3, 5, 9, 13, 3, 5, 9, 13];
        for i in 0..16 {
            let k = idx2[i];
            let s = s2[i];
            let g = md4_g(b, c, d);
            a = rotl(a.wrapping_add(g).wrapping_add(x[k]).wrapping_add(0x5A82_7999), s);
            (a, b, c, d) = (d, a, b, c);
        }

        // half-MD4 跳过第 3 轮

        a = a.wrapping_add(aa);
        b = b.wrapping_add(bb);
        c = c.wrapping_add(cc);
        d = d.wrapping_add(dd);
    }

    // 返回第二/三个字的混合 (ext4 惯例)
    b.wrapping_add(c)
}

// ── HTree 管理器 ──

pub struct Htree {
    dir_ino: Ino,
    root: HtreeRoot,
    seed: u32,
}

impl Htree {
    /// 从目录 inode 加载或创建 HTree
    pub fn load_or_create(dir_ino: Ino) -> FsResult<Self> {
        let mut di = FsDiskInode::empty();
        read_disk_inode(dir_ino, &mut di).map_err(|_| FsError::Eio)?;

        let mut root = HtreeRoot::empty();
        let mut buf = [0u8; HTREE_BLOCK_SIZE as usize];

        if di.size >= HTREE_BLOCK_SIZE {
            // 尝试读取根节点
            let _ = get_ramdisk_device().read_bytes(
                dir_data_offset(dir_ino) + 0, &mut buf[..core::mem::size_of::<HtreeRoot>()],
            );
            unsafe {
                root = core::ptr::read_unaligned(buf.as_ptr() as *const HtreeRoot);
            }
        }

        if root.magic == HTREE_MAGIC && root.entries_count > 0 {
            let seed = root.seed;
            return Ok(Htree { dir_ino, root, seed });
        }

        // 创建新 HTree
        let seed = simple_seed(dir_ino);
        root.init(seed);
        Htree::write_root(dir_ino, &root)?;

        Ok(Htree { dir_ino, root, seed })
    }

    /// 查找目录项的逻辑偏移
    pub fn lookup(&self, name: &str) -> FsResult<Option<u64>> {
        if name.len() > 39 {
            return Ok(None);
        }

        let hash = half_md4(name.as_bytes(), self.seed);

        if self.root.indirect_levels == 0 {
            // 直接从根节点搜索
            self.search_in_node(0, hash, name)
        } else {
            // 遍历间接层
            self.search_indirect(0, self.root.indirect_levels, hash, name)
        }
    }

    /// 插入目录项映射
    pub fn insert(&mut self, name: &str, block_offset: u64) -> FsResult<()> {
        let hash = half_md4(name.as_bytes(), self.seed);

        if self.root.entries_count < self.root.entries_limit {
            // 在根节点中插入
            self.insert_into_node(0, hash, block_offset as u32)?;
            self.root.entries_count += 1;
        } else {
            // 需要分裂: 提升间接层
            self.split_root(hash, block_offset as u32)?;
        }

        Htree::write_root(self.dir_ino, &self.root)?;
        Ok(())
    }

    /// 删除目录项映射
    pub fn remove(&mut self, name: &str, block_offset: u64) -> FsResult<()> {
        let hash = half_md4(name.as_bytes(), self.seed);
        if self.root.indirect_levels == 0 {
            self.remove_from_node(0, hash, block_offset as u32)?;
            if self.root.entries_count > 0 {
                self.root.entries_count -= 1;
            }
        } else {
            self.remove_indirect(0, self.root.indirect_levels, hash, block_offset as u32)?;
            if self.root.entries_count > 0 {
                self.root.entries_count -= 1;
            }
        }
        Htree::write_root(self.dir_ino, &self.root)?;
        Ok(())
    }

    /// 返回条目计数
    pub fn count(&self) -> u16 {
        self.root.entries_count
    }

    // ── 内部方法 ──

    fn search_in_node(&self, node_off: u64, hash: u32, name: &str) -> FsResult<Option<u64>> {
        let mut buf = [0u8; HTREE_BLOCK_SIZE as usize];
        read_htree_block(self.dir_ino, node_off, &mut buf)?;

        let header = unsafe {
            core::ptr::read_unaligned(buf.as_ptr() as *const HtreeNodeHeader)
        };

        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        for i in 0..header.entries_count as usize {
            let off = entry_base + i * entry_size;
            let entry: HtreeEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const HtreeEntry)
            };
            if entry.hash == hash {
                // 验证名称匹配 (处理哈希冲突)
                if let Some(ino) = dir_entry_matches(self.dir_ino, entry.block_offset as u64, name) {
                    return Ok(Some(ino));
                }
            }
        }
        Ok(None)
    }

    fn search_indirect(&self, node_off: u64, levels: u8, hash: u32, name: &str) -> FsResult<Option<u64>> {
        let mut buf = [0u8; HTREE_BLOCK_SIZE as usize];
        read_htree_block(self.dir_ino, node_off, &mut buf)?;

        let header = unsafe {
            core::ptr::read_unaligned(buf.as_ptr() as *const HtreeNodeHeader)
        };
        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        // 二分查找: 找到 hash <= entry.hash 的最后一个条目
        let mut child_off = 0u64;
        for i in 0..header.entries_count as usize {
            let off = entry_base + i * entry_size;
            let entry: HtreeEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const HtreeEntry)
            };
            if hash >= entry.hash {
                child_off = entry.block_offset as u64;
            } else {
                break;
            }
        }

        if child_off == 0 {
            return Ok(None);
        }

        if levels == 1 {
            self.search_in_node(child_off, hash, name)
        } else {
            self.search_indirect(child_off, levels - 1, hash, name)
        }
    }

    fn insert_into_node(&mut self, node_off: u64, hash: u32, block_off: u32) -> FsResult<()> {
        let mut buf = [0u8; HTREE_BLOCK_SIZE as usize];
        if node_off == 0 {
            // 写入根节点区域
            read_htree_block(self.dir_ino, 0, &mut buf)?;
        } else {
            read_htree_block(self.dir_ino, node_off, &mut buf)?;
        }

        let mut header = unsafe {
            core::ptr::read_unaligned(buf.as_ptr() as *const HtreeNodeHeader)
        };

        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        // 按 hash 排序插入
        let mut insert_pos = header.entries_count as usize;
        for i in 0..header.entries_count as usize {
            let off = entry_base + i * entry_size;
            let entry: HtreeEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const HtreeEntry)
            };
            if hash < entry.hash {
                insert_pos = i;
                break;
            }
        }

        // 后移后续条目
        for i in (insert_pos..header.entries_count as usize).rev() {
            let src_off = entry_base + i * entry_size;
            let dst_off = entry_base + (i + 1) * entry_size;
            unsafe {
                let src = buf.as_ptr().add(src_off);
                let dst = buf.as_mut_ptr().add(dst_off);
                core::ptr::copy(src, dst, entry_size);
            }
        }

        // 写入新条目
        let new_entry = HtreeEntry { hash, block_offset: block_off };
        let ins_off = entry_base + insert_pos * entry_size;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr().add(ins_off) as *mut HtreeEntry, new_entry);
        }

        header.entries_count += 1;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr() as *mut HtreeNodeHeader, header);
        }

        write_htree_block(self.dir_ino, node_off, &buf)
    }

    fn remove_from_node(&mut self, node_off: u64, hash: u32, block_off: u32) -> FsResult<()> {
        let mut buf = [0u8; HTREE_BLOCK_SIZE as usize];
        read_htree_block(self.dir_ino, node_off, &mut buf)?;

        let mut header = unsafe {
            core::ptr::read_unaligned(buf.as_ptr() as *const HtreeNodeHeader)
        };
        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        let mut found = false;
        for i in 0..header.entries_count as usize {
            let off = entry_base + i * entry_size;
            let entry: HtreeEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const HtreeEntry)
            };
            if entry.hash == hash && entry.block_offset == block_off {
                found = true;
                // 前移后续条目
                for j in i..header.entries_count as usize - 1 {
                    let src_off = entry_base + (j + 1) * entry_size;
                    let dst_off = entry_base + j * entry_size;
                    unsafe {
                        core::ptr::copy(buf.as_ptr().add(src_off), buf.as_mut_ptr().add(dst_off), entry_size);
                    }
                }
                break;
            }
        }

        if found {
            header.entries_count -= 1;
            unsafe {
                core::ptr::write_unaligned(buf.as_mut_ptr() as *mut HtreeNodeHeader, header);
            }
            write_htree_block(self.dir_ino, node_off, &buf)?;
        }
        Ok(())
    }

    fn remove_indirect(&mut self, node_off: u64, levels: u8, hash: u32, block_off: u32) -> FsResult<()> {
        let buf = [0u8; HTREE_BLOCK_SIZE as usize];
        // 读取然后找子节点
        let mut node_buf = buf;
        read_htree_block(self.dir_ino, node_off, &mut node_buf)?;

        let header = unsafe {
            core::ptr::read_unaligned(node_buf.as_ptr() as *const HtreeNodeHeader)
        };
        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        let mut child_off = 0u64;
        for i in 0..header.entries_count as usize {
            let off = entry_base + i * entry_size;
            let entry: HtreeEntry = unsafe {
                core::ptr::read_unaligned(node_buf.as_ptr().add(off) as *const HtreeEntry)
            };
            if hash >= entry.hash {
                child_off = entry.block_offset as u64;
            }
        }

        if child_off == 0 {
            return Ok(());
        }

        if levels == 1 {
            self.remove_from_node(child_off, hash, block_off)
        } else {
            self.remove_indirect(child_off, levels - 1, hash, block_off)
        }
    }

    fn split_root(&mut self, hash: u32, block_off: u32) -> FsResult<()> {
        // 分配新的内部节点
        let new_node_off = alloc_htree_block(self.dir_ino)?;

        // 将根转为内部节点: 复制现有条目, 分一半到新节点
        let mut root_buf = [0u8; HTREE_BLOCK_SIZE as usize];
        read_htree_block(self.dir_ino, 0, &mut root_buf)?;

        let count = self.root.entries_count;
        let half = count / 2;
        let entry_size = core::mem::size_of::<HtreeEntry>();
        let entry_base = core::mem::size_of::<HtreeNodeHeader>();

        // 创建新叶子节点 (接收后一半条目)
        let mut new_buf = [0u8; HTREE_BLOCK_SIZE as usize];
        let new_header = HtreeNodeHeader {
            entries_count: count - half,
            entries_limit: HTREE_ENTRIES_PER_BLOCK,
            checksum: 0,
        };
        unsafe {
            core::ptr::write_unaligned(new_buf.as_mut_ptr() as *mut HtreeNodeHeader, new_header);
        }
        for i in 0..(count - half) as usize {
            let src_off = entry_base + (half as usize + i) * entry_size;
            let dst_off = entry_base + i * entry_size;
            unsafe {
                core::ptr::copy(root_buf.as_ptr().add(src_off), new_buf.as_mut_ptr().add(dst_off), entry_size);
            }
        }
        write_htree_block(self.dir_ino, new_node_off, &new_buf)?;

        // 获取新节点的第一个条目哈希作为索引键
        let first_new: HtreeEntry = unsafe {
            core::ptr::read_unaligned(new_buf.as_ptr().add(entry_base) as *const HtreeEntry)
        };

        // 更新根为内部节点 (只保留前 half 个条目作为第一个子节点, 加一个指向新节点的索引)
        let new_root_header = HtreeNodeHeader {
            entries_count: half + 1,
            entries_limit: HTREE_ENTRIES_PER_BLOCK,
            checksum: 0,
        };
        unsafe {
            core::ptr::write_unaligned(root_buf.as_mut_ptr() as *mut HtreeNodeHeader, new_root_header);
        }
        // 截断条目到 half 个, 保留前 half
        // 添加指向新节点的索引条目
        let idx_entry = HtreeEntry {
            hash: first_new.hash,
            block_offset: new_node_off as u32,
        };
        let idx_off = entry_base + half as usize * entry_size;
        unsafe {
            core::ptr::write_unaligned(root_buf.as_mut_ptr().add(idx_off) as *mut HtreeEntry, idx_entry);
        }
        write_htree_block(self.dir_ino, 0, &root_buf)?;

        // 插入新条目到合适的叶子
        if hash < first_new.hash {
            self.insert_into_node(0, hash, block_off)?;
        } else {
            self.insert_into_node(new_node_off, hash, block_off)?;
        }

        self.root.indirect_levels = 1;
        self.root.entries_count += 1;
        Ok(())
    }

    fn write_root(dir_ino: Ino, root: &HtreeRoot) -> FsResult<()> {
        // 写入 HTree 根到目录文件偏移 0
        let data_off = dir_data_offset(dir_ino);
        let size = core::mem::size_of::<HtreeRoot>();
        let buf = unsafe {
            core::slice::from_raw_parts(root as *const _ as *const u8, size)
        };
        get_ramdisk_device().write_bytes(data_off, buf).map_err(|_| FsError::Eio)
    }
}

// ── 辅助函数 ──

/// 获取目录文件数据区域的物理偏移
fn dir_data_offset(dir_ino: Ino) -> u64 {
    // 目录数据存储在目录 inode 的扩展树中
    // 对于小目录, 用线性区域回退
    if let Ok(mut tree) = ExtentTree::load(dir_ino) {
        if let Ok(Some((phys, _))) = tree.bmap(0) {
            return phys;
        }
    }
    FS_INODE_TABLE_OFFSET + FS_TOTAL_INODES * FS_INODE_SIZE
}

/// 读取 HTree 块 (目录文件内逻辑偏移)
fn read_htree_block(dir_ino: Ino, logical_off: u64, buf: &mut [u8]) -> FsResult<()> {
    let data_start = dir_data_offset(dir_ino);
    let phys_off = data_start + logical_off;
    get_ramdisk_device().read_bytes(phys_off, buf).map_err(|_| FsError::Eio)
}

/// 写入 HTree 块
fn write_htree_block(dir_ino: Ino, logical_off: u64, buf: &[u8]) -> FsResult<()> {
    let data_start = dir_data_offset(dir_ino);
    let phys_off = data_start + logical_off;
    get_ramdisk_device().write_bytes(phys_off, buf).map_err(|_| FsError::Eio)
}

/// 为 HTree 分配新的 4KB 块
fn alloc_htree_block(dir_ino: Ino) -> FsResult<u64> {
    use crate::fs::fs_fs::space;
    let phys = space::global_space().alloc(HTREE_BLOCK_SIZE, 0)?
        .ok_or(FsError::Enospc)?;
    // 在扩展树中插入映射
    let mut di = FsDiskInode::empty();
    read_disk_inode(dir_ino, &mut di).map_err(|_| FsError::Eio)?;
    let logical_off = di.size;
    if let Ok(mut tree) = ExtentTree::load(dir_ino) {
        let _ = tree.insert(logical_off, HTREE_BLOCK_SIZE, phys, HTREE_BLOCK_SIZE, 0);
    }
    Ok(logical_off)
}

/// 验证目录项名称匹配 (处理 HTree 哈希冲突)
fn dir_entry_matches(dir_ino: Ino, entry_off: u64, name: &str) -> Option<Ino> {
    let mut entry_buf = [0u8; 64];
    let data_start = dir_data_offset(dir_ino);
    if get_ramdisk_device().read_bytes(data_start + entry_off, &mut entry_buf).is_err() {
        return None;
    }
    let entry_ino: u64 = unsafe { core::ptr::read_unaligned(entry_buf.as_ptr() as *const u64) };
    if entry_ino == 0 {
        return None;
    }
    let name_len = unsafe {
        core::ptr::read_unaligned(entry_buf.as_ptr().add(8) as *const u16) as usize
    };
    let entry_name = unsafe {
        let ptr = entry_buf.as_ptr().add(10);
        core::str::from_utf8(core::slice::from_raw_parts(ptr, name_len.min(39)))
    }.unwrap_or("");
    if entry_name == name {
        Some(entry_ino)
    } else {
        None
    }
}

/// 生成简单的哈希种子 (基于 inode 号和当前时间)
fn simple_seed(dir_ino: Ino) -> u32 {
    // 混合 inode 号 + 固定随机值
    let mut h: u32 = dir_ino as u32;
    h ^= 0x9E37_79B9;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h
}
