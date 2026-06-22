// fs/fs_fs/compress.rs — 透明压缩框架 (Phase 9: lz4_flex + ruzstd 解码)
// 支持 noop/lz4 完整, zstd 解压, 与 FsExtent.compression 字段集成
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::error::{FsResult, FsError};

// ── 压缩算法标识 ──

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlg {
    None = 0,
    Zstd = 1,
    Lz4  = 2,
}

impl CompressionAlg {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => CompressionAlg::Zstd,
            2 => CompressionAlg::Lz4,
            _ => CompressionAlg::None,
        }
    }
}

// ── 压缩结果 ──

pub struct CompressResult {
    pub data: *mut u8,
    pub len: usize,
}

impl CompressResult {
    pub fn new(data: *mut u8, len: usize) -> Self {
        CompressResult { data, len }
    }
}

// ── 压缩器 trait ──

pub trait Compressor {
    fn compress(&self, input: &[u8]) -> FsResult<CompressResult>;
    fn decompress(&self, input: &[u8], original_len: usize) -> FsResult<CompressResult>;
    fn algorithm(&self) -> CompressionAlg;
    fn max_compressed_size(&self, input_len: usize) -> usize;
}

// ── 无操作压缩机 ──

pub struct NoopCompressor;

impl Compressor for NoopCompressor {
    fn compress(&self, input: &[u8]) -> FsResult<CompressResult> {
        let ptr = crate::mm::slab::kmalloc(input.len()).ok_or(FsError::Enomem)?;
        unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), ptr.as_ptr(), input.len()); }
        Ok(CompressResult::new(ptr.as_ptr(), input.len()))
    }

    fn decompress(&self, input: &[u8], _original_len: usize) -> FsResult<CompressResult> {
        let ptr = crate::mm::slab::kmalloc(input.len()).ok_or(FsError::Enomem)?;
        unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), ptr.as_ptr(), input.len()); }
        Ok(CompressResult::new(ptr.as_ptr(), input.len()))
    }

    fn algorithm(&self) -> CompressionAlg { CompressionAlg::None }
    fn max_compressed_size(&self, input_len: usize) -> usize { input_len }
}

// ── LZ4 压缩机 (基于 lz4_flex) ──

pub struct Lz4Compressor;

impl Compressor for Lz4Compressor {
    fn compress(&self, input: &[u8]) -> FsResult<CompressResult> {
        let max_size = lz4_flex::block::get_maximum_output_size(input.len());
        let ptr = crate::mm::slab::kmalloc(max_size).ok_or(FsError::Enomem)?;
        let compressed = lz4_flex::block::compress_into(input, unsafe {
            core::slice::from_raw_parts_mut(ptr.as_ptr(), max_size)
        }).map_err(|_| FsError::Eio)?;
        Ok(CompressResult::new(ptr.as_ptr(), compressed))
    }

    fn decompress(&self, input: &[u8], original_len: usize) -> FsResult<CompressResult> {
        let ptr = crate::mm::slab::kmalloc(original_len).ok_or(FsError::Enomem)?;
        lz4_flex::block::decompress_into(input, unsafe {
            core::slice::from_raw_parts_mut(ptr.as_ptr(), original_len)
        }).map_err(|_| FsError::Eio)?;
        Ok(CompressResult::new(ptr.as_ptr(), original_len))
    }

    fn algorithm(&self) -> CompressionAlg { CompressionAlg::Lz4 }
    fn max_compressed_size(&self, input_len: usize) -> usize {
        lz4_flex::block::get_maximum_output_size(input_len)
    }
}

// ── Zstd 压缩机 (基于 ruzstd 解码器; 压缩为桩, 需 C 库链接) ──

pub struct ZstdCompressor;

impl Compressor for ZstdCompressor {
    fn compress(&self, _input: &[u8]) -> FsResult<CompressResult> {
        // ruzstd 是纯解码器库, 不支持编码
        // zstd 压缩需要外部 C 库 (zstd-sys/libzstd) 或完整 Rust 编码器
        Err(FsError::Enosys)
    }

    fn decompress(&self, input: &[u8], _original_len: usize) -> FsResult<CompressResult> {
        // 使用 ruzstd::StreamingDecoder 进行流式解压
        use ruzstd::{decoding::StreamingDecoder, io_nostd::Read};
        let mut decoder = StreamingDecoder::new(input).map_err(|_| FsError::Eio)?;

        // 分配输出缓冲 (最多 16MB 解压后数据)
        let max_out = 16 * 1024 * 1024;
        let ptr = crate::mm::slab::kmalloc(max_out).ok_or(FsError::Enomem)?;
        let dest = unsafe { core::slice::from_raw_parts_mut(ptr.as_ptr(), max_out) };
        let mut written = 0usize;

        loop {
            let remaining = &mut dest[written..];
            if remaining.is_empty() { break; }
            match decoder.read(remaining) {
                Ok(0) => break,
                Ok(n) => written += n,
                Err(_) => {
                    unsafe { crate::mm::slab::kfree(ptr); }
                    return Err(FsError::Eio);
                }
            }
        }

        Ok(CompressResult::new(ptr.as_ptr(), written))
    }

    fn algorithm(&self) -> CompressionAlg { CompressionAlg::Zstd }
    fn max_compressed_size(&self, input_len: usize) -> usize {
        input_len + (input_len >> 7) + 128
    }
}

// ── 全局压缩机 ──

static NOOP: NoopCompressor = NoopCompressor;
static ZSTD: ZstdCompressor = ZstdCompressor;
static LZ4: Lz4Compressor = Lz4Compressor;

pub fn get_compressor(alg: CompressionAlg) -> &'static dyn Compressor {
    match alg {
        CompressionAlg::None => &NOOP,
        CompressionAlg::Zstd => &ZSTD,
        CompressionAlg::Lz4  => &LZ4,
    }
}

// ── 便捷 API ──

pub fn compress_page(input: &[u8], alg: CompressionAlg) -> FsResult<CompressResult> {
    let compressor = get_compressor(alg);
    compressor.compress(input)
}

pub fn decompress_page(input: &[u8], original_len: usize, alg: CompressionAlg) -> FsResult<CompressResult> {
    let compressor = get_compressor(alg);
    compressor.decompress(input, original_len)
}

pub fn free_compress_result(result: CompressResult) {
    if result.len > 0 && !result.data.is_null() {
        if let Some(ptr) = core::ptr::NonNull::new(result.data) {
            unsafe { crate::mm::slab::kfree(ptr); }
        }
    }
}
