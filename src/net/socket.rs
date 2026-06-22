// net/socket.rs — BSD 套接字接口 (AF_INET, AF_INET6, AF_UNIX, AF_PACKET)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;

pub const AF_UNIX: u16 = 1;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;
pub const AF_PACKET: u16 = 17;

pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

pub const SOL_SOCKET: i32 = 1;
pub const SO_REUSEADDR: i32 = 2;
pub const SO_KEEPALIVE: i32 = 9;
pub const SO_BINDTODEVICE: i32 = 25;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: [u8; 4],
    pub sin_zero: [u8; 8],
}

#[repr(C)]
pub struct Socket {
    pub fd: i32,
    pub family: u16,
    pub sock_type: i32,
    pub protocol: i32,
    pub state: SocketState,
    pub local_addr: Option<SockAddrIn>,
    pub remote_addr: Option<SockAddrIn>,
    pub recv_buf: [u8; 65536],
    pub recv_len: usize,
    pub send_buf: [u8; 65536],
    pub send_len: usize,
    pub flags: u32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SocketState {
    Closed = 0,
    Created = 1,
    Bound = 2,
    Listening = 3,
    Connected = 4,
}

const MAX_SOCKETS: usize = 128;

pub struct SocketTable {
    pub sockets: [Option<Socket>; MAX_SOCKETS],
    pub count: usize,
}

impl SocketTable {
    pub const fn new() -> Self {
        const NONE: Option<Socket> = None;
        SocketTable { sockets: [NONE; MAX_SOCKETS], count: 0 }
    }

    pub fn alloc(&mut self, family: u16, sock_type: i32, protocol: i32) -> Option<i32> {
        for i in 0..MAX_SOCKETS {
            if self.sockets[i].is_none() {
                self.sockets[i] = Some(Socket {
                    fd: i as i32,
                    family,
                    sock_type,
                    protocol,
                    state: SocketState::Created,
                    local_addr: None,
                    remote_addr: None,
                    recv_buf: [0u8; 65536],
                    recv_len: 0,
                    send_buf: [0u8; 65536],
                    send_len: 0,
                    flags: 0,
                });
                self.count += 1;
                return Some(i as i32);
            }
        }
        None
    }
}

static SOCKET_TABLE: SpinLock<SocketTable> = SpinLock::new(SocketTable::new());

pub fn socket(family: u16, sock_type: i32, protocol: i32) -> i32 {
    SOCKET_TABLE.lock().alloc(family, sock_type, protocol).unwrap_or(-1)
}

pub fn bind(_fd: i32, _addr: &SockAddrIn) -> i32 { -1 }
pub fn listen(_fd: i32, _backlog: i32) -> i32 { -1 }
pub fn accept(_fd: i32) -> i32 { -1 }
pub fn connect(_fd: i32, _addr: &SockAddrIn) -> i32 { -1 }
pub fn send(_fd: i32, _buf: &[u8], _flags: i32) -> isize { -1 }
pub fn recv(_fd: i32, _buf: &mut [u8], _flags: i32) -> isize { -1 }
