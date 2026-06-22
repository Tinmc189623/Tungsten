# FreeType 移植模块

本目录为 **移植组件**（非 Tungsten 自研），供内核 `build.rs` 编译字体引擎。

- `config/` — Tungsten 裸机适配层
- `upstream/` — FreeType 构建所需最小 C 源码子集（不含 HTML 文档与 autotools）

完整上游包可在本地通过 TungstenOS 构建树获取；公开仓库仅同步编译所需文件。

FreeType 采用 [FreeType License (FTL)](https://www.freetype.org/license.html)。
详见 `upstream/LICENSE.TXT`。

Copyright © 2026 Nexsteaduser. All Rights Reserved.
