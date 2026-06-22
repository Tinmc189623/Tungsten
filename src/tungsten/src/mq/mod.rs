// mq/mod.rs — POSIX 消息队列 (mq_open/mq_send/mq_receive)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub const MQ_MAXMSG: usize = 256; pub const MQ_MAXSIZE: usize = 8192;
#[repr(C)] pub struct MqMessage { pub priority: u32, pub data: [u8; MQ_MAXSIZE], pub len: usize }
pub struct MqDescriptor { pub name: [u8; 64], pub flags: i32, pub maxmsg: usize, pub msgsize: usize, pub curmsgs: usize, pub msgs: [Option<MqMessage>; MQ_MAXMSG], pub head: usize, pub tail: usize }
pub struct MqTable { pub queues: [Option<MqDescriptor>; 64], pub count: usize }
static MQ_TABLE: SpinLock<MqTable> = SpinLock::new(MqTable { queues: [const { None }; 64], count: 0 });
pub fn init() { crate::serial::write_str(b"mq: ready\n"); }
pub fn sys_mq_open(_name: *const u8, _flags: i32, _mode: i32) -> i32 { -1 }
pub fn sys_mq_send(_fd: i32, _msg: *const u8, _len: usize, _prio: u32) -> i32 { -1 }
pub fn sys_mq_receive(_fd: i32, _msg: *mut u8, _len: usize, _prio: *mut u32) -> isize { -1 }
