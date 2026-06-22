// build.rs — Tungsten 内核构建脚本
// 编译 FreeType C 源码 + Zig HAL 为 x86_64 裸机静态库
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    inject_version_from_ver_json(&manifest_dir);

    let zig = find_zig();

    // ── 编译 FreeType ──
    build_freetype(&manifest_dir, &out_dir, &zig);

    // ── 编译 Zig HAL ──
    build_hal(&manifest_dir, &out_dir, &zig);
}

/// 定位 ver.json：优先 crate 目录，其次 TungstenOS 工程根
fn resolve_ver_json(manifest_dir: &PathBuf) -> PathBuf {
    let local = manifest_dir.join("ver.json");
    if local.is_file() {
        return local;
    }
    manifest_dir.join("../../ver.json")
}

/// 从 ver.json 注入版本环境变量供 version.rs 使用
fn inject_version_from_ver_json(manifest_dir: &PathBuf) {
    let ver_path = resolve_ver_json(manifest_dir);
    let text = std::fs::read_to_string(&ver_path)
        .unwrap_or_else(|_| panic!("无法读取 ver.json: {}", ver_path.display()));

    let field = |key: &str| -> String {
        json_string_field(&text, key).unwrap_or_else(|| panic!("ver.json 缺少字段: {key}"))
    };

    let kernel = field("KERNEL_VERSION");
    let os = field("OS_VERSION");
    let api = field("API_VERSION");
    let knr_date = field("KNR_BUILD_DATE");
    let os_date = field("OS_BUILD_DATE");
    let knr_patch = field("KERNEL_PATCH_VERSION");
    let os_patch = field("OS_PATCH_VERSION");

    let parts: Vec<&str> = kernel.split('.').collect();
    let major = parts.first().copied().unwrap_or("0");
    let minor = parts.get(1).copied().unwrap_or("0");
    let patch = parts.get(2).copied().unwrap_or("0");

    println!("cargo:rerun-if-changed={}", ver_path.display());
    let local_ver = manifest_dir.join("ver.json");
    if local_ver.is_file() {
        println!("cargo:rerun-if-changed={}", local_ver.display());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let version_rs = format!(
        r#"// 由 build.rs 根据 ver.json 自动生成，请勿手改
pub const VERSION_MAJOR: u32 = {major};
pub const VERSION_MINOR: u32 = {minor};
pub const VERSION_PATCH: u32 = {patch};
pub const VERSION: &str = "{kernel}";
pub const DISPLAY: &str = "Tungsten v{kernel}";
pub const OS_VERSION: &str = "{os}";
pub const OS_DISPLAY: &str = "TungstenOS v{os}";
pub const API_VERSION: &str = "{api}";
pub const KERNEL_PATCH_VERSION: &str = "{knr_patch}";
pub const OS_PATCH_VERSION: &str = "{os_patch}";
pub const KNR_BUILD_DATE: &str = "{knr_date}";
pub const OS_BUILD_DATE: &str = "{os_date}";
"#
    );
    std::fs::write(out_dir.join("tungsten_version.rs"), version_rs)
        .expect("写入 tungsten_version.rs 失败");
}

/// 从 JSON 文本提取字符串字段（简单解析，避免 build 依赖 serde）
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\": \"");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 编译 FreeType 为裸机静态库（源码位于 src/modules/freetype）
fn build_freetype(manifest_dir: &PathBuf, out_dir: &PathBuf, zig: &PathBuf) {
    let src_root = manifest_dir.join("..");
    let freetype_src = src_root.join("modules/freetype/upstream");
    let kernel_cfg = src_root.join("modules/freetype/config");

    // FreeType 可能尚未下载，跳过编译并生成空库
    if !freetype_src.exists() {
        eprintln!("cargo:warning=FreeType 源码未找到，跳过 FreeType 编译");
        return;
    }

    let include_flags = vec![
        format!("-I{}", kernel_cfg.display()),
        format!("-I{}", freetype_src.join("include").display()),
    ];

    let cflags: Vec<String> = vec![
        "-target".into(),
        "x86_64-freestanding".into(),
        "-DFT2_BUILD_LIBRARY".into(),
        "-DFT_CONFIG_OPTION_DISABLE_STREAM_SUPPORT".into(),
        "-ffreestanding".into(),
        "-fPIC".into(),
        "-O2".into(),
        "-fno-stack-protector".into(),
        "-fno-strict-aliasing".into(),
        "-Wno-unused-parameter".into(),
        // 禁用 AVX/AVX2/AVX-512，裸机内核不提供 XSAVE/XRSTOR 支持
        "-mno-avx".into(),
        "-mno-avx2".into(),
        "-mno-avx512f".into(),
    ];

    let sources: Vec<PathBuf> = vec![
        kernel_cfg.join("ftsystem.c"),
        kernel_cfg.join("ftalloc.c"),
        kernel_cfg.join("ftdebug.c"),
        kernel_cfg.join("ft_rust_helpers.c"),
        freetype_src.join("src/base/ftbase.c"),
        freetype_src.join("src/base/ftinit.c"),
        freetype_src.join("src/base/ftbitmap.c"),
        freetype_src.join("src/base/ftmm.c"),
        freetype_src.join("src/sfnt/sfnt.c"),
        freetype_src.join("src/truetype/truetype.c"),
        freetype_src.join("src/smooth/smooth.c"),
        freetype_src.join("src/autofit/autofit.c"),
        freetype_src.join("src/cff/cff.c"),
        freetype_src.join("src/psaux/psaux.c"),
        freetype_src.join("src/psnames/psnames.c"),
    ];

    let mut objects: Vec<PathBuf> = Vec::new();

    for src in &sources {
        if !src.exists() {
            eprintln!("cargo:warning=跳过不存在的 FreeType 源文件: {}", src.display());
            continue;
        }
        let stem = src.file_stem().unwrap().to_str().unwrap();
        let obj = out_dir.join(format!("freetype_{}.o", stem));

        let mut cmd = Command::new(zig);
        cmd.arg("cc");
        cmd.args(&cflags);
        cmd.args(&include_flags);
        cmd.arg("-c");
        cmd.arg(src);
        cmd.arg("-o");
        cmd.arg(&obj);

        let status = cmd.status().expect("无法运行 zig cc");
        assert!(status.success(), "zig cc 编译失败: {}", src.display());

        objects.push(obj);
    }

    if objects.is_empty() {
        eprintln!("cargo:warning=没有编译任何 FreeType 源文件");
        return;
    }

    // 使用 zig ar 创建静态库
    let lib_a = out_dir.join("libfreetype_tungsten.a");
    let mut ar = Command::new(zig);
    ar.arg("ar");
    ar.arg("rcs");
    ar.arg(&lib_a);
    for obj in &objects {
        ar.arg(obj);
    }
    let status = ar.status().expect("无法运行 zig ar");
    assert!(status.success(), "zig ar 创建 FreeType 库失败");

    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=freetype_tungsten");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../modules/freetype/config/");
    println!("cargo:rerun-if-changed=../modules/freetype/upstream/");
}

/// 编译 Zig HAL 为裸机静态库
fn build_hal(manifest_dir: &PathBuf, out_dir: &PathBuf, zig: &PathBuf) {
    let hal_dir = manifest_dir.join("../hal");
    let hal_lib = out_dir.join("libhal_tungsten.a");
    let hal_target = "x86_64-freestanding";
    let hal_optimize = if profile() == "release" { "ReleaseFast" } else { "Debug" };

    // 所有 HAL 源文件
    let hal_sources = [
        "ioport.zig",
        "serial.zig",
        "framebuffer.zig",
        "pci.zig",
        "timer.zig",
        "interrupt.zig",
        "dma.zig",
    ];

    let mut hal_objects: Vec<PathBuf> = Vec::new();

    for src_name in &hal_sources {
        let src_path = hal_dir.join(src_name);
        if !src_path.exists() {
            eprintln!("cargo:warning=HAL 源文件不存在，跳过: {}", src_path.display());
            continue;
        }

        let stem = src_name.trim_end_matches(".zig");
        let obj = out_dir.join(format!("hal_{}.o", stem));

        let mut cmd = Command::new(zig);
        cmd.arg("build-obj");
        cmd.arg("-target").arg(hal_target);
        cmd.arg("-O").arg(hal_optimize);
        cmd.arg("-fPIC");
        cmd.arg("-fno-stack-protector");
        cmd.arg("--name").arg(format!("hal_{}", stem));
        cmd.arg(&src_path);
        cmd.current_dir(out_dir);

        let status = cmd.status()
            .unwrap_or_else(|_| panic!("无法运行 zig build-obj 编译 {}", src_name));
        assert!(status.success(), "zig build-obj 失败: {}", src_name);

        hal_objects.push(obj);
    }

    if hal_objects.is_empty() {
        eprintln!("cargo:warning=没有编译任何 HAL 源文件");
        return;
    }

    // 用 zig ar 创建静态库
    let mut hal_ar = Command::new(zig);
    hal_ar.arg("ar");
    hal_ar.arg("rcs");
    hal_ar.arg(&hal_lib);
    for obj in &hal_objects {
        hal_ar.arg(obj);
    }
    let ar_status = hal_ar.status().expect("无法运行 zig ar 创建 HAL 库");
    assert!(ar_status.success(), "zig ar 创建 HAL 库失败");

    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=hal_tungsten");
    println!("cargo:rerun-if-changed=../hal/");
}

/// 返回当前 cargo profile
fn profile() -> &'static str {
    match std::env::var("PROFILE").as_deref() {
        Ok("release") => "release",
        _ => "debug",
    }
}

/// 查找 Zig 编译器路径
fn find_zig() -> PathBuf {
    // 优先检查常见 Windows 安装路径
    let candidates = [
        "C:\\zig\\zig.exe",
        "C:\\zig\\zig",
        "C:/zig/zig.exe",
        "C:/zig/zig",
    ];
    for path in &candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }
    // 从 PATH 查找
    let output = Command::new("where")
        .arg("zig")
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            String::from_utf8(o.stdout).ok()
                .map(|s| s.lines().next().unwrap_or("zig").to_string())
        } else { None });
    if let Some(path) = output {
        return PathBuf::from(path.trim());
    }
    // 回退到 PATH 中的 zig
    PathBuf::from("zig")
}
