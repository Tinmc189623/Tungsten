# Tungsten

**Tungsten™** — 面向 x86_64 的四层特权级架构内核（Ring 0 ~ Ring 3），由 Nexsteaduser 以 **GPL v3** 开源发布。

| 字段 | 值 |
|------|-----|
| 内核版本 `KERNEL_VERSION` | **0.2.2** |
| 内核补丁 `KERNEL_PATCH_VERSION` | KP-20260623-0022-001 |
| 内核构建日 `KERNEL_BUILD_DATE` | 20260623 |
| 关联系统 `OS_VERSION` | 0.2 |
| 系统补丁 `OS_PATCH_VERSION` | OS-20260623-0002-001 |
| TAPI `API_VERSION` | 0.2 |
| 官网 | [tungsten-kernel.org](https://tungsten-kernel.org/) |

本仓库为 **Tungsten 内核 Rust 源码**（`src/tungsten` crate），不含 TungstenOS 闭源系统层。

## 架构

```
Ring 0 ─ 内核本体：调度、内存、syscall、核心服务
Ring 1 ─ 驱动与可加载模块
Ring 2 ─ I/O 子系统 / 文件系统（服务层接口）
Ring 3 ─ 用户程序（由 TungstenOS 承载）
```

统一通过 **@TAPI (Tungsten-API)** 访问系统能力；引导协议基于 **Limine**。

## 目录

| 路径 | 说明 |
|------|------|
| `src/` | 内核模块（arch、mm、sched、fs、net、service 等） |
| `build.rs` | 构建脚本（读取 `ver.json`、链接 HAL / FreeType） |
| `Cargo.toml` | Rust 包清单 |
| `ver.json` | 版本与补丁元数据（与 TungstenOS 主树同步） |

## 构建

在完整 TungstenOS 工程树中：

```bash
make hal      # Zig HAL
make kernel   # 本 crate → ELF
```

单独在本目录编译（需已安装 Zig、Rust nightly、`x86_64-unknown-none` target，以及同级 HAL / FreeType 路径）：

```bash
cargo build --target x86_64-unknown-none --release -Z build-std=core,alloc
```

产物：`target/x86_64-unknown-none/release/tungsten`

## 版本

版本号由根目录 `ver.json` 驱动，`build.rs` 在编译时生成 `src/version.rs` 所用常量。修改版本请编辑 `ver.json` 后重新 `cargo build`。

## 许可

本仓库内核源码采用 **GNU General Public License v3.0 or later**。详见 [LICENSE](LICENSE)。

Copyright © 2026 Nexsteaduser. All Rights Reserved.

Tinmc189623 · Nexlyh · admin@nexsteaduser.com
