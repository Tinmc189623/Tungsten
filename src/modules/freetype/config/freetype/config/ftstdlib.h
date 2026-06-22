/*
 * ftstdlib.h — FreeType C 标准库映射（Tungsten 内核裸机适配）
 * 将所有 C 标准库依赖映射到 __builtin_* 内建函数
 * 与本地 setjmp/longjmp 存根
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

#ifndef FTSTDLIB_H_
#define FTSTDLIB_H_

#include <stddef.h>
#include <stdint.h>
#include <limits.h>

/* ======================== 整数类型 ======================== */
#define FT_UINT_MAX     UINT_MAX
#define FT_INT_MAX      INT_MAX
#define FT_INT_MIN      INT_MIN
#define FT_ULONG_MAX    ULONG_MAX
#define FT_CHAR_BIT     CHAR_BIT
#define FT_LONG_MAX     LONG_MAX
#define FT_LONG_MIN     LONG_MIN
#define FT_USHORT_MAX   65535U

/* ptrdiff_t — 某些编译器中 stddef.h 可能不提供此 typedef */
#ifndef __ptrdiff_t_defined
typedef __PTRDIFF_TYPE__  ft_ptrdiff_t;
#else
typedef ptrdiff_t         ft_ptrdiff_t;
#endif

/* ======================== 内存操作 ======================== */
#define ft_memchr(buf, c, n)     __builtin_memchr(buf, c, n)
#define ft_memcmp(p1, p2, n)     __builtin_memcmp(p1, p2, n)
#define ft_memcpy(dst, src, n)   __builtin_memcpy(dst, src, n)
#define ft_memmove(dst, src, n)  __builtin_memmove(dst, src, n)
#define ft_memset(buf, c, n)     __builtin_memset(buf, c, n)

/* ======================== 字符串操作（裸机环境内联实现） ======================== */
static inline char *ft_strcat(char *dst, const char *src) {
    char *p = dst;
    while (*p) p++;
    while ((*p++ = *src++));
    return dst;
}
static inline int ft_strcmp(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}
static inline char *ft_strcpy(char *dst, const char *src) {
    char *p = dst;
    while ((*p++ = *src++));
    return dst;
}
static inline unsigned long ft_strlen(const char *s) {
    const char *p = s;
    while (*p) p++;
    return (unsigned long)(p - s);
}
static inline int ft_strncmp(const char *a, const char *b, unsigned long n) {
    while (n && *a && *a == *b) { a++; b++; n--; }
    if (!n) return 0;
    return (unsigned char)*a - (unsigned char)*b;
}
static inline char *ft_strncpy(char *dst, const char *src, unsigned long n) {
    char *p = dst;
    while (n && (*p++ = *src++)) n--;
    while (n--) *p++ = '\0';
    return dst;
}
static inline char *ft_strrchr(const char *s, int c) {
    const char *found = 0;
    do { if (*s == (char)c) found = s; } while (*s++);
    return (char *)found;
}
/* ft_strstr — 简单实现 */
static inline char *ft_strstr(const char *s, const char *sub) {
    if (!*sub) return (char *)s;
    while (*s) {
        const char *a = s, *b = sub;
        while (*a && *b && *a == *b) { a++; b++; }
        if (!*b) return (char *)s;
        s++;
    }
    return (char *)0;
}
/* ft_strtol — 简单实现（__builtin_strtol 不存在于 clang） */
static inline long ft_strtol(const char *s, char **ep, int base) {
    long r = 0; int sign = 1;
    while (*s == ' ' || *s == '\t') s++;
    if      (*s == '-') { sign = -1; s++; }
    else if (*s == '+') { s++; }
    if (base == 0) {
        if (*s == '0' && (s[1] == 'x' || s[1] == 'X')) { base = 16; s += 2; }
        else if (*s == '0') { base = 8; s++; }
        else { base = 10; }
    } else if (base == 16 && *s == '0' && (s[1] == 'x' || s[1] == 'X')) { s += 2; }
    while (1) {
        int d; char c = *s;
        if      (c >= '0' && c <= '9') d = c - '0';
        else if (c >= 'a' && c <= 'f') d = c - 'a' + 10;
        else if (c >= 'A' && c <= 'F') d = c - 'A' + 10;
        else break;
        if (d >= base) break;
        r = r * base + d; s++;
    }
    if (ep) *ep = (char *)s;
    return r * sign;
}

/* ======================== 字符分类 ======================== */
/* 注意: ftobjs.h 内部也会定义 ft_isdigit/ft_isupper/ft_islower/ft_isalpha/ft_isalnum,
 * 因此此处定义的会与内部定义冲突, 我们移除冲突项, 让 ftobjs.h 的版本生效 */
#define ft_isprint(c)   ((unsigned char)(c) >= 0x20 && (unsigned char)(c) <= 0x7E)
#define ft_isspace(c)   ((c) == ' ' || (c) == '\t' || (c) == '\n' || (c) == '\r')
#define ft_tolower(c)   ((c) >= 'A' && (c) <= 'Z' ? (c) + 32 : (c))
#define ft_toupper(c)   ((c) >= 'a' && (c) <= 'z' ? (c) - 32 : (c))

/* ======================== 数值转换 ======================== */
#define ft_abs(x)       ((x) < 0 ? -(x) : (x))
#define ft_atoi(s)      ((int)ft_strtol(s, (char **)0, 10))

/* ======================== 错误处理存根 ======================== */
/* FreeType 使用 setjmp/longjmp 实现 TRY/CATCH 错误恢复。
 * 内核上下文中，FreeType 错误视为致命错误，无需恢复，
 * 因此设 setjmp 始终返回 0（永不执行恢复），longjmp 为空操作。 */
#define ft_setjmp(env)           0
#define ft_longjmp(env, val)     ((void)(val), (void)0)

/* jmp_buf 类型定义 — 必须为数组类型供 FreeType 内部声明使用 */
typedef int                       ft_jmp_buf[1];

/* ======================== 文件操作存根（内核中不使用文件 I/O） ======================== */
#define ft_fclose(f)             ((void)0)
#define ft_fopen(p, m)           ((void *)0)
#define ft_fread(b, s, n, f)     ((unsigned long)0)
#define ft_fseek(f, o, w)        (-1)
#define ft_ftell(f)              ((long)-1)
#define ft_feof(f)               (1)
#define ft_ferror(f)             (1)
#define ft_fputs(s, f)           (-1)
#define ft_remove(p)             (-1)
#define ft_rename(o, n)          (-1)
#define ft_sprintf(buf, fmt, ...) 0
#define ft_vfprintf(f, fmt, ap)  (-1)
#define ft_vsnprintf(b, m, f, a) 0
#define ft_fprintf(f, ...)       (-1)

/* ======================== 退出存根 ======================== */
#define ft_exit(s)              do { for (;;); } while (0)
#define ft_qsort(b, n, s, c)    ((void)0)

/* FreeType 内部未通过 ftstdlib.h 引用 ft_getenv，但某些代码路径可能需要 */
#define ft_getenv(n)            ((char *)0)

#endif /* FTSTDLIB_H_ */
