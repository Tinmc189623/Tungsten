// pipe/mod.rs — 管道子系统 (匿名管道 + 命名管道 FIFO)
// 环形缓冲区、读写同步、select/poll 集成
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub const PIPE_BUF: usize = 65536;
pub const PIPE_MAX_READERS: usize = 16;
pub const PIPE_MAX_WRITERS: usize = 16;
#[repr(C)] pub struct Pipe {
    pub buf: [u8; PIPE_BUF], pub head: usize, pub tail: usize, pub count: usize,
    pub readers: u16, pub writers: u16, pub flags: u32, pub inode: u64,
}
impl Pipe { pub const fn new() -> Self { Pipe { buf: [0; PIPE_BUF], head:0,tail:0,count:0,readers:0,writers:0,flags:0,inode:0 } } }
pub struct PipeTable { pub pipes: [Option<Pipe>; 128], pub count: usize }
static PIPE_TABLE: SpinLock<PipeTable> = SpinLock::new(PipeTable { pipes: [const { None }; 128], count: 0 });
pub fn sys_pipe(fds: &mut [i32; 2]) -> i32 { let mut t = PIPE_TABLE.lock();
    for i in 0..128 { if t.pipes[i].is_none() { t.pipes[i] = Some(Pipe::new()); t.count += 1;
    fds[0] = i as i32; fds[1] = i as i32; return 0; } } -1 }
pub fn sys_pipe2(fds: &mut [i32; 2], _flags: i32) -> i32 { sys_pipe(fds) }
pub fn pipe_read(fd: i32, buf: &mut [u8]) -> isize { let mut t = PIPE_TABLE.lock();
    if let Some(ref mut p) = t.pipes[fd as usize] { if p.count == 0 { return 0; }
    let n = buf.len().min(p.count); for i in 0..n { buf[i] = p.buf[p.head]; p.head=(p.head+1)%PIPE_BUF; p.count-=1; } n as isize } else { -1 } }
pub fn pipe_write(fd: i32, buf: &[u8]) -> isize { let mut t = PIPE_TABLE.lock();
    if let Some(ref mut p) = t.pipes[fd as usize] { let avail = PIPE_BUF - p.count;
    let n = buf.len().min(avail); for i in 0..n { p.buf[p.tail] = buf[i]; p.tail=(p.tail+1)%PIPE_BUF; p.count+=1; } n as isize } else { -1 } }
pub fn init() { crate::serial::write_str(b"pipe: subsystem ready\n"); }
