/*
 * ftdebug.c — FreeType 调试输出存根（Tungsten 内核裸机适配）
 * 内核中禁用 FreeType 的调试消息。
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#include <ft2build.h>
#include FT_FREETYPE_H

#if defined(FT_DEBUG_LEVEL_ERROR) || defined(FT_DEBUG_LEVEL_TRACE)

#include <stdarg.h>

/* 编译时启用调试：输出到串口（暂留接口，当前为空） */
void FT_Message(const char *fmt, ...) {
    (void)fmt;
}

void FT_Panic(const char *fmt, ...) {
    (void)fmt;
    for (;;);
}

void FT_DumpMemory(void) {
}

#endif /* FT_DEBUG_LEVEL_ERROR || FT_DEBUG_LEVEL_TRACE */

/* FT_Trace_Disable / FT_Trace_Enable — 始终编译，smooth 渲染器需要 */
void FT_Trace_Disable(void) { }
void FT_Trace_Enable(void)  { }
