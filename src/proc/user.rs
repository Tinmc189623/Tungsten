// user.rs — 内核用户账户表（多用户身份）
// 内置 root / tungsten / guest，供 login、su、权限检查使用
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;

/// 单条用户记录
#[derive(Clone, Copy)]
pub struct UserAccount {
    pub uid: u32,
    pub gid: u32,
    pub name: [u8; 32],
    pub name_len: u8,
    pub home: [u8; 64],
    pub home_len: u8,
}

impl UserAccount {
    /// 构造用户记录
    const fn empty() -> Self {
        UserAccount {
            uid: 0,
            gid: 0,
            name: [0; 32],
            name_len: 0,
            home: [0; 64],
            home_len: 0,
        }
    }

    /// 用户名切片
    pub fn name_str(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    /// 主目录路径切片
    pub fn home_str(&self) -> &[u8] {
        &self.home[..self.home_len as usize]
    }
}

const MAX_USERS: usize = 32;

struct UserTable {
    users: [UserAccount; MAX_USERS],
    count: usize,
}

static USER_TABLE: SpinLock<UserTable> = SpinLock::new(UserTable {
    users: [UserAccount::empty(); MAX_USERS],
    count: 0,
});

/// 写入定长 ASCII 字段
fn copy_ascii(dst: &mut [u8], src: &[u8]) -> u8 {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n as u8
}

/// 注册内置用户账户
fn register(uid: u32, gid: u32, name: &[u8], home: &[u8]) {
    let mut table = USER_TABLE.lock();
    if table.count >= MAX_USERS {
        return;
    }
    let mut acc = UserAccount::empty();
    acc.uid = uid;
    acc.gid = gid;
    acc.name_len = copy_ascii(&mut acc.name, name);
    acc.home_len = copy_ascii(&mut acc.home, home);
    let idx = table.count;
    table.users[idx] = acc;
    table.count += 1;
}

/// 初始化用户表
pub fn init() {
    register(0, 0, b"root", b"/root");
    register(1000, 1000, b"tungsten", b"/Users/tungsten");
    register(1001, 1001, b"guest", b"/Users/guest");
    crate::serial::write_str(b"user: ");
    crate::serial_put_u64(user_count() as u64);
    crate::serial::write_str(b" accounts loaded\n");
}

/// 已注册用户数
pub fn user_count() -> usize {
    USER_TABLE.lock().count
}

/// 按 UID 查找用户
pub fn lookup_uid(uid: u32) -> Option<UserAccount> {
    let table = USER_TABLE.lock();
    for i in 0..table.count {
        if table.users[i].uid == uid {
            return Some(table.users[i]);
        }
    }
    None
}

/// 按用户名查找 (uid, gid)
pub fn lookup_name(name: &str) -> Option<(u32, u32)> {
    let table = USER_TABLE.lock();
    for i in 0..table.count {
        let u = &table.users[i];
        let uname = core::str::from_utf8(u.name_str()).ok()?;
        if uname == name {
            return Some((u.uid, u.gid));
        }
    }
    None
}

/// 将用户列表写入缓冲区（每行: uid name home）
pub fn format_users(buf: &mut [u8]) -> usize {
    let table = USER_TABLE.lock();
    let mut pos = 0;
    for i in 0..table.count {
        let u = &table.users[i];
        let line = format_user_line(u);
        if pos + line.len() + 1 > buf.len() {
            break;
        }
        buf[pos..pos + line.len()].copy_from_slice(&line);
        pos += line.len();
        buf[pos] = b'\n';
        pos += 1;
    }
    pos
}

/// 格式化单行用户信息
fn format_user_line(u: &UserAccount) -> [u8; 96] {
    let mut out = [0u8; 96];
    let mut p = 0;
    p += write_u32(&mut out[p..], u.uid);
    out[p] = b' ';
    p += 1;
    let name = u.name_str();
    out[p..p + name.len()].copy_from_slice(name);
    p += name.len();
    out[p] = b' ';
    p += 1;
    let home = u.home_str();
    out[p..p + home.len()].copy_from_slice(home);
    p += home.len();
    out[..p].try_into().unwrap_or([0u8; 96])
}

/// 将 u32 写入 ASCII 缓冲区，返回写入字节数
fn write_u32(buf: &mut [u8], val: u32) -> usize {
    let mut tmp = [0u8; 10];
    let mut n = val;
    let mut len = 0;
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    len
}
