# Tungsten

**Tungsten™** 四层特权级架构内核（Ring 0 / Ring 1），x86_64，版本 **0.2.2**。

| 项目 | 版本 | 官网 |
|------|------|------|
| Tungsten™ | 0.2.2 | [tungsten-kernel.org](https://tungsten-kernel.org/) |
| TungstenOS | 0.2 | [tungstenos.com](https://tungstenos.com/) |

本仓库**仅包含 Tungsten 内核源码**（Rust + Zig HAL + FreeType 移植子集）。TungstenOS 操作系统层为闭源商业组件，不在此公开。

> FreeType 为移植依赖，在语言统计中标记为 `linguist-vendored`；仓库主体为 **Rust** 内核实现。

## 架构

```
Ring 0 ─ Tungsten 本体、内存管理、调度、syscall  (Rust)
Ring 1 ─ 驱动、可加载模块                        (Rust)
         Zig HAL ─ IOPort、PCI、串口、帧缓冲等硬件抽象
```

## 构建

依赖：Rust nightly、`x86_64-unknown-none` target、Zig、Ruby。

```bash
ruby scripts/install_deps.rb --check   # 可选：检查依赖
make hal                               # 编译 Zig HAL
make kernel                            # 编译 Tungsten ELF 内核
make all                               # hal + kernel
make clean                             # 清理产物
```

内核产物：`src/tungsten/target/x86_64-unknown-none/release/tungsten`（ELF）

## 许可

Tungsten 内核采用 **GPL v3** 开源许可。详见 [LICENSE](LICENSE)。

Copyright © 2026 Nexsteaduser. All Rights Reserved.

*Tinmc189623 / Nexlyh*
