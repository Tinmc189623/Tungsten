/*
 * ftoption.h — FreeType 选项覆盖（Tungsten 内核裸机适配）
 * 基于原始 ftoption.h，通过 #include_next 继承并禁用多余功能。
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */
#include_next <freetype/config/ftoption.h>

/* 禁用 zlib/gzip — 内核中不需要 WOFF 解压缩 */
#undef FT_CONFIG_OPTION_USE_ZLIB

/* 禁用 BDF 表格支持 — 内核不需要嵌入式位图 */
#undef TT_CONFIG_OPTION_BDF
