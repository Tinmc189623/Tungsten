// net/tcp.rs — TCP 传输控制协议 (RFC 793)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub const TCP_STATE_CLOSED: u8 = 0;
pub const TCP_STATE_LISTEN: u8 = 1;
pub const TCP_STATE_SYN_SENT: u8 = 2;
pub const TCP_STATE_SYN_RCVD: u8 = 3;
pub const TCP_STATE_ESTABLISHED: u8 = 4;
pub const TCP_STATE_FIN_WAIT1: u8 = 5;
pub const TCP_STATE_FIN_WAIT2: u8 = 6;
pub const TCP_STATE_CLOSE_WAIT: u8 = 7;
pub const TCP_STATE_CLOSING: u8 = 8;
pub const TCP_STATE_LAST_ACK: u8 = 9;
pub const TCP_STATE_TIME_WAIT: u8 = 10;

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_PSH: u8 = 0x08;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const TCP_FLAG_URG: u8 = 0x20;

#[repr(C, packed)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub data_offset: u8,
    pub flags: u8,
    pub window: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
}

#[repr(C)]
pub struct TcpCb {
    pub state: u8,
    pub src_port: u16,
    pub dst_port: u16,
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub snd_wnd: u16,
    pub rcv_nxt: u32,
    pub rcv_wnd: u16,
    pub iss: u32,
    pub irs: u32,
    pub retransmit_timer: u64,
    pub mss: u16,
    pub srtt: u32,
    pub rttvar: u32,
    pub rto: u32,
}

impl TcpCb {
    pub const fn new() -> Self {
        TcpCb {
            state: TCP_STATE_CLOSED,
            src_port: 0, dst_port: 0,
            snd_una: 0, snd_nxt: 0, snd_wnd: 0,
            rcv_nxt: 0, rcv_wnd: 65535,
            iss: 0, irs: 0,
            retransmit_timer: 0, mss: 1460,
            srtt: 0, rttvar: 0, rto: 3000,
        }
    }
}

pub fn init() {}
