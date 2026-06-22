// main.rs — Tungsten 内核入口与早期初始化（Limine 引导）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

use tungsten::arch;
use tungsten::audio;
use tungsten::backtrace;
use tungsten::cpu;
use tungsten::console;
use tungsten::devices;
use tungsten::devices::input;
use tungsten::drm;
use tungsten::fs;
use tungsten::ipc;
use tungsten::limine_boot;
use tungsten::mm;
use tungsten::net;
use tungsten::proc;
use tungsten::sched;
use tungsten::security;
use tungsten::serial;
use tungsten::usb;
use tungsten::uxiloader;
use tungsten::virtio;
use tungsten::version;
use tungsten::block;
use tungsten::crypto;
use tungsten::tty;
use tungsten::pipe;
use tungsten::shm;
use tungsten::mq;
use tungsten::sem;
use tungsten::timer;
use tungsten::power;
use tungsten::smp;
use tungsten::kmod;
use tungsten::ptrace;
use tungsten::cgroup;
use tungsten::bpf;
use tungsten::kvm;
use tungsten::watchdog;
use tungsten::random;
use tungsten::service;

/// 早期引导栈（BSS 段内，Limine 自动映射）
#[used]
#[unsafe(link_section = ".bss")]
static mut EARLY_STACK: [u8; 32768] = [0u8; 32768];

core::arch::global_asm!(
    ".pushsection .text._start, \"ax\"",
    ".globl _start",
    "_start:",
    "lea rsp, [rip + {stack} + {size}]",
    "xor rbp, rbp",
    "cld",
    /* 早期串口标记：输出 '0K' 表示栈已就绪 */
    "mov dx, 0x3f8",
    "mov al, 48",
    "out dx, al",
    "mov al, 75",
    "out dx, al",
    /* 保存参数，输出 '1' 后跳转 Rust 主函数 */
    "push rdi",
    "push rsi",
    "mov al, 49",
    "out dx, al",
    "call rust_main",
    /* 不应到达此处 */
    "mov al, 88",
    "out dx, al",
    "cli",
    "hlt",
    ".popsection",
    stack = sym EARLY_STACK,
    size = const core::mem::size_of::<[u8; 32768]>(),
);

/// 早期串口输出宏（直接 I/O 端口，不依赖 Zig HAL）
macro_rules! early_serial {
    ($c:expr) => {
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") $c) }
    };
}

/// Rust 主入口（Limine 已设置长模式、页表）
#[unsafe(no_mangle)]
pub extern "C" fn rust_main(_magic: u64, _boot_info_ptr: u64) -> ! {
    early_serial!(b'A');

    // 从 Limine 请求响应构建 BootInfo
    let boot_info = limine_boot::build_boot_info();
    early_serial!(b'B');

    // CPU 特性检测 (CPUID)
    cpu::detect();
    early_serial!(b'C');

    // GDT 初始化（含 Ring 0-3 段描述符 + TSS）
    arch::gdt::init();
    early_serial!(b'D');

    // IDT 初始化（256 项异常/中断向量）
    arch::idt::init();
    early_serial!(b'E');

    // 启用 SSE/SSE2 — x86_64 baseline 要求，FreeType/Rust 编译器均依赖
    arch::x86_64::enable_sse();
    early_serial!(b'F');
    // 串口初始化（通过 Zig HAL FFI）
    serial::init();
    early_serial!(b'G');
    serial::write_str(version::DISPLAY.as_bytes());
    serial::write_str(b" [serial]\n");

    // 控制台初始化（帧缓冲）
    serial::write_str(b"step: console init...\n");
    unsafe { console::init(&boot_info); }
    serial::write_str(b"step: console done\n");

    // 物理内存管理器
    serial::write_str(b"step: pmm init...\n");
    unsafe { mm::pmm::init(&boot_info); }
    serial::write_str(b"step: pmm done\n");

    // 虚拟内存管理器
    serial::write_str(b"step: vmm init...\n");
    unsafe { mm::vmm::init(&boot_info); }

    // SLAB 分配器
    serial::write_str(b"step: slab init...\n");
    unsafe { mm::slab::init(); }
    mm::slab::init_fs_caches();
    serial::write_str(b"step: slab done\n");

    // APIC 初始化
    serial::write_str(b"step: acpi init...\n");
    unsafe { arch::x86_64::acpi::init(boot_info.rsdp_addr); }
    serial::write_str(b"step: apic init...\n");
    unsafe { arch::apic::init(boot_info.rsdp_addr); }
    serial::write_str(b"step: apic done\n");

    // 系统调用接口
    serial::write_str(b"step: syscall init...\n");
    arch::x86_64::syscall::init();
    serial::write_str(b"step: syscall done\n");

    // 输出内核 Banner
    serial::write_str(b"step: banner...\n");
    serial::write_str(version::DISPLAY.as_bytes());
    serial::write_str(b" starting...\n");

    serial::write_str(b"fb: ");
    serial_put_u64(boot_info.fb_width);
    serial::write_str(b"x");
    serial_put_u64(boot_info.fb_height);
    serial::write_str(b" @ ");
    serial_put_u64(boot_info.fb_bpp as u64);
    serial::write_str(b"bpp\n");

    serial::write_str(b"fb addr: ");
    serial_put_u64_hex(boot_info.fb_addr);
    serial::write_str(b"\n");

    serial::write_str(b"mmap entries: ");
    serial_put_u64(boot_info.mmap_entries);
    serial::write_str(b"\n");

    // 帧缓冲输出 Banner
    serial::write_str(b"fb: console writes...\n");
    console::write_str(version::DISPLAY);
    console::write_str(" [framebuffer]\n");

    // 调度器（任务在 service::bootstrap_platform 中创建）
    serial::write_str(b"step: sched init...\n");
    sched::init();

    // IPC 初始化
    serial::write_str(b"step: ipc init...\n");
    ipc::init();
    serial::write_str(b"step: ipc done\n");

    // 设备框架初始化
    serial::write_str(b"step: devices init...\n");
    devices::init();
    serial::write_str(b"step: devices done (");
    serial_put_u64(devices::device_count() as u64);
    serial::write_str(b" devs)\n");

    // 网络子系统
    serial::write_str(b"step: net init...\n");
    net::init();
    serial::write_str(b"step: net done\n");

    // 音频子系统
    serial::write_str(b"step: audio init...\n");
    audio::init();
    serial::write_str(b"step: audio done\n");

    // USB 子系统
    serial::write_str(b"step: usb init...\n");
    usb::init();
    serial::write_str(b"step: usb done\n");

    // VirtIO 半虚拟化驱动
    serial::write_str(b"step: virtio init...\n");
    virtio::init();
    serial::write_str(b"step: virtio done\n");

    // 安全子系统
    serial::write_str(b"step: security init...\n");
    security::init();
    serial::write_str(b"step: security done\n");

    // 块设备子系统
    serial::write_str(b"step: block init...\n");
    block::init();
    serial::write_str(b"step: block done\n");
    // 加密子系统
    serial::write_str(b"step: crypto init...\n");
    crypto::init();
    serial::write_str(b"step: crypto done\n");
    // TTY 终端
    serial::write_str(b"step: tty init...\n");
    tty::init();
    serial::write_str(b"step: tty done\n");
    // 管道
    pipe::init();
    // 共享内存
    shm::init();
    // 消息队列
    mq::init();
    // 信号量
    sem::init();
    // 高精度定时器
    serial::write_str(b"step: timer init...\n");
    timer::init();
    serial::write_str(b"step: timer done\n");
    // 电源管理
    power::init();
    // SMP 多核
    smp::init();
    // 内核模块
    kmod::init();
    // ptrace
    ptrace::init();
    // cgroup
    cgroup::init();
    // eBPF
    bpf::init();
    // KVM 虚拟化
    kvm::init();
    // 看门狗
    watchdog::init();
    // 随机数
    random::init();
    // DRM/KMS 显示框架
    serial::write_str(b"step: drm init...\n");
    drm::init();
    serial::write_str(b"step: drm done\n");
    // 进程管理器
    serial::write_str(b"step: proc init...\n");
    proc::init();
    serial::write_str(b"step: proc done\n");
    service::init();
    serial::write_str(b"step: service registry done\n");
    // VFS + FS 文件系统初始化
    serial::write_str(b"step: vfs init...\n");
    fs::init();
    serial::write_str(b"step: vfs done\n");

    // 创建测试文件验证 FS 功能
    serial::write_str(b"step: create test file...\n");
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'1') };
    let test_fd = fs::sys_open("/hello.txt", 0x41); // O_CREAT | O_WRONLY
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'2') };
    if test_fd >= 0 {
        fs::sys_write(test_fd, version::OS_DISPLAY.as_bytes());
        fs::sys_write(test_fd, b"\n");
        fs::sys_close(test_fd);
        serial::write_str(b"step: created /hello.txt\n");
    } else {
        serial::write_str(b"step: failed to create /hello.txt\n");
    }

    // 加载 OS 层（从 Limine 模块）
    serial::write_str(b"step: loading OS layer...\n");
    if let Some(os_data) = limine_boot::get_os_module() {
        serial::write_str(b"step: OS module found, size=");
        serial_put_u64(os_data.len() as u64);
        serial::write_str(b"\n");
        unsafe {
            if let Some(os_prog) = uxiloader::load_uxi_direct(os_data) {
                serial::write_str(b"step: OS loaded, base=0x");
                serial_put_u64_hex(os_prog.base);
                serial::write_str(b" entry=0x");
                serial_put_u64_hex(os_prog.entry);
                serial::write_str(b"\n");

                // 注册 OS 层入口，由 os_layer 服务线程执行
                serial::write_str(b"step: register OS entry for os_layer service...\n");
                let os_entry: extern "C" fn() = core::mem::transmute(os_prog.entry as *const ());
                service::set_os_layer_entry(os_entry);
            } else {
                serial::write_str(b"step: OS module invalid format\n");
            }
        }
    } else {
        serial::write_str(b"step: OS module not found, continuing\n");
    }

    serial::write_str(b"step: enter platform service runtime...\n");
    console::write_str("\n");
    console::write_str(version::DISPLAY);
    console::write_str(" \u{2014} Platform Services Online\n");
    service::bootstrap_platform();
}

/// 内核串口 Shell 任务（shelld 服务线程）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel_shell_task() -> ! {
    shell_loop();
}

/// 将 u64 格式化为十进制 ASCII 并输出到串口
fn serial_put_u64(val: u64) {
    let mut buf = [0u8; 20];
    if val == 0 {
        serial::write_str(b"0");
        return;
    }
    let mut n = val;
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    serial::write_str(&buf[i..]);
}

/// 将 u64 格式化为十六进制 ASCII 并输出到串口
fn serial_put_u64_hex(val: u64) {
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> ((15 - i) * 4)) & 0xF) as u8;
        buf[i + 2] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
    }
    serial::write_str(&buf);
}

/// 串口命令行循环
fn shell_loop() -> ! {
    let mut line = [0u8; 256];
    loop {
        service::app::cooperative_point();
        serial::write_str(b"tungsten> ");
        let len = serial::read_line(&mut line);
        if len == 0 { continue; }

        let cmd = core::str::from_utf8(&line[..len]).unwrap_or("");
        match cmd {
            "help" | "?" => cmd_help(),
            "version" | "ver" => cmd_version(),
            "echo" => { serial::write_str(b"echo: usage: echo <text>\n"); }
            "clear" | "cls" => cmd_clear(),
            "info" => cmd_info(),
            "ls" => cmd_ls(),
            "ps" => cmd_ps(),
            "services" => cmd_services(),
            "svc" => cmd_svc(""),
            "reboot" => cmd_reboot(),
            "kbd" => cmd_kbd(),
            "layout" => cmd_layout(),
            "kb-shell" => { keyboard_shell(); }
            "sched" => {
                serial::write_str(b"starting scheduler...\n");
                sched::start();
            }
            "uname" => cmd_uname(),
            "whoami" => cmd_whoami(),
            "users" => cmd_users(),
            "hostname" => serial::write_str(b"tungsten\n"),
            "uptime" => cmd_uptime(),
            "date" => cmd_date(),
            "free" => cmd_free(),
            "df" => cmd_df(),
            "stat" => cmd_stat(),
            "devlist" => cmd_devlist(),
            "blklist" => cmd_blklist(),
            "cpus" => cmd_cpus(),
            "modlist" => cmd_modlist(),
            "mount" => cmd_mount(),
            "umount" => cmd_umount(""),
            "env" => cmd_env(),
            _ => {
                if cmd.starts_with("echo ") {
                    serial::write_str(cmd[5..].as_bytes());
                    serial::write_str(b"\n");
                } else if cmd.starts_with("cat ") {
                    cmd_cat(&cmd[4..]);
                } else if cmd.starts_with("mkdir ") {
                    cmd_mkdir(&cmd[6..]);
                } else if cmd.starts_with("rm ") {
                    cmd_rm(&cmd[3..]);
                } else if cmd.starts_with("write ") {
                    cmd_write_file(&cmd[6..]);
                } else if cmd.starts_with("layout ") {
                    cmd_layout_set(&cmd[7..]);
                } else if cmd.starts_with("mv ") {
                    cmd_mv(&cmd[3..]);
                } else if cmd.starts_with("modload ") {
                    cmd_modload(&cmd[8..]);
                } else if cmd.starts_with("umount ") {
                    cmd_umount(&cmd[7..]);
                } else if cmd.starts_with("login ") {
                    cmd_login(&cmd[6..]);
                } else if cmd.starts_with("su ") {
                    cmd_su(&cmd[3..]);
                } else if cmd.starts_with("svc ") {
                    cmd_svc(&cmd[4..]);
                } else if cmd.starts_with("time ") {
                    cmd_time(&cmd[5..]);
                } else {
                    serial::write_str(b"unknown: ");
                    serial::write_str(cmd.as_bytes());
                    serial::write_str(b"\n");
                }
            }
        }
        service::app::cooperative_point();
    }
}

/// 显示帮助信息
fn cmd_help() {
    serial::write_str(version::DISPLAY.as_bytes());
    serial::write_str(b" commands:\n");
    serial::write_str(b"  help, ?           - this help\n");
    serial::write_str(b"  version, ver      - kernel version info\n");
    serial::write_str(b"  uname             - system identification\n");
    serial::write_str(b"  echo <text>       - echo text\n");
    serial::write_str(b"  layout [name]     - show/set keyboard layout\n");
    serial::write_str(b"  clear, cls        - clear framebuffer\n");
    serial::write_str(b"  info              - system info\n");
    serial::write_str(b"  ls                - list files\n");
    serial::write_str(b"  cat <file>        - show file content\n");
    serial::write_str(b"  mkdir <dir>       - create directory\n");
    serial::write_str(b"  rm <file>         - remove file\n");
    serial::write_str(b"  mv <old> <new>    - rename/move file\n");
    serial::write_str(b"  write <f> <text>  - write text to file\n");
    serial::write_str(b"  ps                - list tasks\n");
    serial::write_str(b"  services          - platform + app services\n");
    serial::write_str(b"  svc start <path>  - start .uxi app service\n");
    serial::write_str(b"  svc stop <name>   - stop app service\n");
    serial::write_str(b"  free              - memory usage\n");
    serial::write_str(b"  df                - disk usage\n");
    serial::write_str(b"  stat              - system statistics\n");
    serial::write_str(b"  uptime            - system uptime\n");
    serial::write_str(b"  date              - current date\n");
    serial::write_str(b"  whoami            - current user (uid)\n");
    serial::write_str(b"  users             - list user accounts\n");
    serial::write_str(b"  login <user>      - switch to user\n");
    serial::write_str(b"  su <user>         - switch effective user\n");
    serial::write_str(b"  hostname          - system hostname\n");
    serial::write_str(b"  env               - environment variables\n");
    serial::write_str(b"  devlist           - list devices\n");
    serial::write_str(b"  blklist           - list block devices\n");
    serial::write_str(b"  cpus              - list CPUs (SMP)\n");
    serial::write_str(b"  modlist           - list kernel modules\n");
    serial::write_str(b"  modload <name>    - load kernel module\n");
    serial::write_str(b"  mount             - list mounts\n");
    serial::write_str(b"  umount <path>     - unmount filesystem\n");
    serial::write_str(b"  kb-shell          - keyboard-driven shell\n");
    serial::write_str(b"  kbd               - test PS/2 keyboard\n");
    serial::write_str(b"  reboot            - reboot system\n");
    serial::write_str(b"  sched             - (already running)\n");
}

/// 显示版本信息
fn cmd_version() {
    serial::write_str(version::DISPLAY.as_bytes());
    serial::write_str(b" (x86_64)\n");
    serial::write_str(b"Kernel: Tungsten ");
    serial::write_str(version::VERSION.as_bytes());
    serial::write_str(b"\n");
    serial::write_str(b"Arch:   x86_64\n");
    serial::write_str(b"License: GPL v3\n");
    serial::write_str(b"Copyright (C) 2026 Nexsteaduser. All rights reserved.\n");
}

/// 系统标识信息
fn cmd_uname() {
    serial::write_str(b"Tungsten tungsten ");
    serial::write_str(version::VERSION.as_bytes());
    serial::write_str(b" x86_64 Nexsteaduser\n");
}

/// 清屏
fn cmd_clear() {
    unsafe { console::clear(); }
    serial::write_str(b"cleared\n");
}

/// 系统信息
fn cmd_info() {
    serial::write_str(b"fb:  ");
    serial_put_u64(unsafe { crate::console::width() } as u64);
    serial::write_str(b"x");
    serial_put_u64(unsafe { crate::console::height() } as u64);
    serial::write_str(b"\n");
    serial::write_str(b"ramdisk: ");
    serial_put_u64(crate::fs::DATA_REGION_START);
    serial::write_str(b" blocks\n");
}

/// 列出根目录文件
fn cmd_ls() {
    let mut buf = [0u8; 1024];
    let written = crate::fs::sys_list_dir("/", &mut buf);
    serial::write_str(&buf[..written]);
    if written == 0 {
        serial::write_str(b"(empty)\n");
    }
}

/// 查看文件内容
fn cmd_cat(path: &str) {
    let path = path.trim();
    if path.is_empty() { serial::write_str(b"usage: cat <path>\n"); return; }
    let fd = crate::fs::sys_open(path, 0);
    if fd < 0 {
        serial::write_str(b"cat: file not found\n");
        return;
    }
    let mut buf = [0u8; 512];
    let n = crate::fs::sys_read(fd, &mut buf);
    if n > 0 {
        serial::write_str(&buf[..n as usize]);
    }
    serial::write_str(b"\n");
    crate::fs::sys_close(fd);
}

/// 创建目录
fn cmd_mkdir(path: &str) {
    let path = path.trim();
    if path.is_empty() { serial::write_str(b"usage: mkdir <path>\n"); return; }
    let ret = crate::fs::sys_mkdir(path);
    if ret < 0 {
        serial::write_str(b"mkdir: failed (err=");
        serial_put_u64((-ret) as u64);
        serial::write_str(b")\n");
    } else {
        serial::write_str(b"mkdir: ok\n");
    }
}

/// 删除文件
fn cmd_rm(path: &str) {
    let path = path.trim();
    if path.is_empty() { serial::write_str(b"usage: rm <path>\n"); return; }
    let ret = crate::fs::sys_unlink(path);
    if ret < 0 {
        serial::write_str(b"rm: failed (err=");
        serial_put_u64((-ret) as u64);
        serial::write_str(b")\n");
    } else {
        serial::write_str(b"rm: ok\n");
    }
}

/// 移动/重命名文件
fn cmd_mv(args: &str) {
    let args = args.trim();
    if let Some(space) = args.find(' ') {
        let old = args[..space].trim();
        let new = args[space + 1..].trim();
        let ret = crate::fs::sys_rename(old, new);
        if ret < 0 {
            serial::write_str(b"mv: failed (err=");
            serial_put_u64((-ret) as u64);
            serial::write_str(b")\n");
        } else {
            serial::write_str(b"mv: ok\n");
        }
    } else {
        serial::write_str(b"usage: mv <old> <new>\n");
    }
}

/// 写入文件
fn cmd_write_file(args: &str) {
    let args = args.trim();
    if let Some(space) = args.find(' ') {
        let path = &args[..space];
        let text = args[space + 1..].trim();
        let fd = crate::fs::sys_open(path, 0x41);
        if fd < 0 {
            serial::write_str(b"write: failed to open\n");
            return;
        }
        crate::fs::sys_write(fd, text.as_bytes());
        crate::fs::sys_close(fd);
        serial::write_str(b"write: ok\n");
    } else {
        serial::write_str(b"usage: write <file> <text>\n");
    }
}

/// 列出进程/线程（多任务）
fn cmd_ps() {
    let mut buf = [0u8; 512];
    let n = proc::format_ps(&mut buf);
    serial::write_str(&buf[..n]);
    if n == 0 {
        serial::write_str(b"(no tasks)\n");
    }
}

/// 列出平台服务状态
fn cmd_services() {
    let mut buf = [0u8; 1536];
    let n = service::format_all(&mut buf);
    serial::write_str(&buf[..n]);
    if n == 0 {
        serial::write_str(b"(no services)\n");
    }
}

/// 应用服务控制
fn cmd_svc(args: &str) {
    let args = args.trim();
    if args.is_empty() || args == "list" {
        cmd_services();
        return;
    }
    if let Some(path) = args.strip_prefix("start ") {
        let path = path.trim();
        let ret = service::start_app(path);
        if ret < 0 {
            serial::write_str(b"svc: start failed err=");
            serial_put_u64((-ret) as u64);
            serial::write_str(b"\n");
        } else {
            serial::write_str(b"svc: started\n");
        }
        return;
    }
    if let Some(name) = args.strip_prefix("stop ") {
        if service::stop_app(name.trim()) {
            serial::write_str(b"svc: stop ok\n");
        } else {
            serial::write_str(b"svc: not found\n");
        }
        return;
    }
    serial::write_str(b"svc: usage: svc [list|start <path>|stop <name>]\n");
}

/// 当前登录用户
fn cmd_whoami() {
    let mut name = [0u8; 32];
    let n = proc::current_username(&mut name);
    serial::write_str(&name[..n]);
    serial::write_str(b" (uid=");
    serial_put_u64(proc::sys_geteuid() as u64);
    serial::write_str(b")\n");
}

/// 列出系统用户
fn cmd_users() {
    let mut buf = [0u8; 256];
    let n = proc::user::format_users(&mut buf);
    serial::write_str(&buf[..n]);
}

/// 切换登录用户
fn cmd_login(user: &str) {
    let user = user.trim();
    if proc::set_credentials_by_name(user) {
        serial::write_str(b"login: ok as ");
        cmd_whoami();
    } else {
        serial::write_str(b"login: unknown user\n");
    }
}

/// 切换有效用户（同 login，保留会话）
fn cmd_su(user: &str) {
    cmd_login(user);
}

/// 内存使用情况
fn cmd_free() {
    let (total, used, free) = crate::mm::pmm::memory_stats();
    serial::write_str(b"Memory: total=");
    serial_put_u64(total / 1024);
    serial::write_str(b"KB used=");
    serial_put_u64(used / 1024);
    serial::write_str(b"KB free=");
    serial_put_u64(free / 1024);
    serial::write_str(b"KB\n");
}

/// 磁盘使用情况
fn cmd_df() {
    let (total, used) = crate::fs::disk_stats();
    serial::write_str(b"FS: total=");
    serial_put_u64(total);
    serial::write_str(b" blocks, used=");
    serial_put_u64(used);
    serial::write_str(b" blocks\n");
}

/// 系统统计
fn cmd_stat() {
    serial::write_str(b"devices: ");
    serial_put_u64(crate::devices::device_count() as u64);
    serial::write_str(b"\ntasks: ");
    serial_put_u64(crate::sched::task_count() as u64);
    serial::write_str(b"\n");
}

/// 系统运行时间
fn cmd_uptime() {
    serial::write_str(b"uptime: ");
    serial_put_u64(crate::sched::uptime_ticks());
    serial::write_str(b" ticks\n");
}

/// 当前日期（CMOS RTC）
fn cmd_date() {
    let mut buf = [0u8; 32];
    let n = power::acpi_pm::format_rtc(&mut buf);
    serial::write_str(&buf[..n]);
}

/// 环境变量
fn cmd_env() {
    serial::write_str(b"KERNEL=Tungsten\n");
    serial::write_str(b"VERSION=");
    serial::write_str(version::VERSION.as_bytes());
    serial::write_str(b"\n");
    serial::write_str(b"ARCH=x86_64\n");
    serial::write_str(b"SHELL=/bin/zsh\n");
    serial::write_str(b"HOME=/Users/root\n");
}

/// 列出设备
fn cmd_devlist() {
    serial::write_str(b"Devices:\n");
    serial::write_str(b"  [0] serial/com1  (0x3F8)\n");
    serial::write_str(b"  [1] ps2/keyboard (0x60)\n");
    serial::write_str(b"  [2] framebuffer  (linear)\n");
}

/// 列出块设备
fn cmd_blklist() {
    let mut buf = [0u8; 2048];
    let n = block::list_devices(&mut buf);
    if n == 0 {
        serial::write_str(b"Block devices: (none)\n");
    } else {
        serial::write_str(b"Block devices:\n");
        serial::write_str(&buf[..n]);
    }
}

/// 列出 CPU（SMP）
fn cmd_cpus() {
    let mut buf = [0u8; 1024];
    let n = smp::list_cpus(&mut buf);
    serial::write_str(b"CPUs:\n");
    if n > 0 {
        serial::write_str(&buf[..n]);
    }
}

/// 列出内核模块
fn cmd_modlist() {
    let mut buf = [0u8; 1024];
    let n = kmod::list(&mut buf);
    if n == 0 {
        serial::write_str(b"Kernel modules: (none loaded)\n");
    } else {
        serial::write_str(b"Kernel modules:\n");
        serial::write_str(&buf[..n]);
    }
}

/// 加载内核模块
fn cmd_modload(name: &str) {
    let name = name.trim();
    if name.is_empty() {
        serial::write_str(b"usage: modload <name>\n");
        return;
    }
    let ret = kmod::load(name);
    if ret < 0 {
        serial::write_str(b"modload: failed (err=");
        serial_put_u64((-ret) as u64);
        serial::write_str(b")\n");
    } else {
        serial::write_str(b"modload: ok\n");
    }
}

/// 列出挂载点
fn cmd_mount() {
    let mut buf = [0u8; 512];
    let n = fs::sys_list_mounts(&mut buf);
    if n == 0 {
        serial::write_str(b"/  -> FS (ramdisk)\n");
    } else {
        serial::write_str(&buf[..n]);
    }
}

/// 卸载文件系统
fn cmd_umount(path: &str) {
    let path = path.trim();
    if path.is_empty() {
        serial::write_str(b"usage: umount <path>\n");
        return;
    }
    let ret = fs::sys_umount(path);
    if ret < 0 {
        serial::write_str(b"umount: failed\n");
    } else {
        serial::write_str(b"umount: ok\n");
    }
}

/// 计时执行命令
fn cmd_time(cmd: &str) {
    let cmd = cmd.trim();
    let t0 = timer::uptime_ms();
    serial::write_str(b"time: ");
    serial::write_str(cmd.as_bytes());
    serial::write_str(b"\n");
    let t1 = timer::uptime_ms();
    serial::write_str(b"elapsed: ");
    serial_put_u64(t1.saturating_sub(t0));
    serial::write_str(b" ms\n");
}

/// 重启系统
fn cmd_reboot() -> ! {
    power::sys_reboot(0);
}

/// 键盘测试命令
fn cmd_kbd() {
    serial::write_str(b"Keyboard test mode. Press keys (ESC to exit)\n");
    loop {
        crate::devices::input::ps2::drain();
        while let Some(ev) = crate::devices::input::read_event() {
            use crate::devices::input::{EventType, keycode};
            let name = keycode::keycode_name(ev.key);
            match ev.event_type {
                EventType::KeyPress => {
                    serial::write_str(b"[kbd] press:  ");
                    serial::write_str(name.as_bytes());
                    if ev.key == keycode::KEY_ESCAPE {
                        serial::write_str(b"\n[ESC] exiting kbd test\n");
                        return;
                    }
                }
                EventType::KeyRelease => {
                    serial::write_str(b"[kbd] release: ");
                    serial::write_str(name.as_bytes());
                }
            }
            serial::write_str(b"\n");
        }
    }
}

/// 键盘驱动命令行输入
fn keyboard_read_line(buf: &mut [u8]) -> usize {
    let mut i = 0usize;
    loop {
        input::ps2::drain();
        if let Some(ev) = input::read_event() {
            if input::process_event(&ev) { continue; }
            if ev.event_type != input::EventType::KeyPress { continue; }
            let caps = input::caps_lock_state();
            let shift = (ev.modifiers & (input::keycode::MOD_LSHIFT | input::keycode::MOD_RSHIFT)) != 0;
            match input::keycode::keycode_to_ascii(ev.key, caps, shift) {
                Some(b'\n') => { serial::write_str(b"\n"); break; }
                Some(b'\x7f') => {
                    if i > 0 { i -= 1; serial::write_str(b"\x08 \x08"); }
                }
                Some(c) if c >= 0x20 && i < buf.len() - 1 => {
                    buf[i] = c; i += 1; serial::write_str(&[c]);
                }
                _ => {}
            }
        }
    }
    buf[i] = 0; i
}

/// 键盘驱动的 Shell
fn keyboard_shell() -> ! {
    let mut line = [0u8; 256];
    serial::write_str(b"\nKeyboard shell (");
    serial::write_str(input::layout::LAYOUT_NAMES[input::layout::current_layout() as usize].as_bytes());
    serial::write_str(b" layout)\n");
    loop {
        serial::write_str(b"tungsten> ");
        let len = keyboard_read_line(&mut line);
        if len == 0 { continue; }
        let cmd = core::str::from_utf8(&line[..len]).unwrap_or("");
        match cmd {
            "help" | "?" => cmd_help(),
            "version" | "ver" => cmd_version(),
            "clear" | "cls" => cmd_clear(),
            "info" => cmd_info(),
            "ls" => cmd_ls(),
            "ps" => cmd_ps(),
            "reboot" => cmd_reboot(),
            "layout" => cmd_layout(),
            "kbd" => cmd_kbd(),
            "uname" => cmd_uname(),
            "whoami" => cmd_whoami(),
            "users" => cmd_users(),
            "hostname" => serial::write_str(b"tungsten\n"),
            "free" => cmd_free(),
            "df" => cmd_df(),
            "uptime" => cmd_uptime(),
            "date" => cmd_date(),
            "env" => cmd_env(),
            "devlist" => cmd_devlist(),
            "blklist" => cmd_blklist(),
            "cpus" => cmd_cpus(),
            "modlist" => cmd_modlist(),
            "stat" => cmd_stat(),
            "mount" => cmd_mount(),
            "shell" => { serial::write_str(b"already in keyboard shell\n"); }
            "sched" => { serial::write_str(b"starting scheduler...\n"); sched::start(); }
            _ => {
                if cmd.starts_with("layout ") { cmd_layout_set(&cmd[7..]); }
                else if cmd.starts_with("echo ") { serial::write_str(cmd[5..].as_bytes()); serial::write_str(b"\n"); }
                else if cmd.starts_with("cat ") { cmd_cat(&cmd[4..]); }
                else if cmd.starts_with("write ") { cmd_write_file(&cmd[6..]); }
                else { serial::write_str(b"unknown: "); serial::write_str(cmd.as_bytes()); serial::write_str(b"\n"); }
            }
        }
    }
}

/// 显示当前键盘布局
fn cmd_layout() {
    input::layout::print_layout();
    serial::write_str(b"layouts: us, uk, de, fr, jp, cn\n");
}

/// 设置键盘布局
fn cmd_layout_set(name: &str) {
    if let Some(l) = input::layout::layout_by_name(name.trim()) {
        input::layout::set_layout(l);
        serial::write_str(b"ok\n");
    } else {
        serial::write_str(b"unknown layout, use: us, uk, de, fr, jp, cn\n");
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial::write_str(b"\n=== KERNEL PANIC ===\n");
    if let Some(loc) = info.location() {
        serial::write_str(b"  at "); serial::write_str(loc.file().as_bytes());
        serial::write_str(b":"); serial_put_u64(loc.line() as u64);
        serial::write_str(b"\n");
    }
    serial::write_str(b"  msg: ");
    use core::fmt::Write;
    struct FmtWriter;
    impl Write for FmtWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            serial::write_str(s.as_bytes());
            Ok(())
        }
    }
    let _ = write!(FmtWriter, "{}", info.message());
    serial::write_str(b"\n");
    unsafe { backtrace::capture(); }
    backtrace::print_backtrace();
    loop { core::hint::spin_loop() }
}
