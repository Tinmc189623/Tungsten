// audio/mod.rs — Tungsten 音频子系统
// HDA/AC97 驱动框架、混音器、PCM 接口
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod hda;
pub mod ac97;
pub mod mixer;
pub mod pcm;

use crate::sync::SpinLock;

/* ── 音频格式 ── */

pub const PCM_S8: u8 = 0;
pub const PCM_U8: u8 = 1;
pub const PCM_S16LE: u8 = 2;
pub const PCM_S16BE: u8 = 3;
pub const PCM_S24LE: u8 = 4;
pub const PCM_S32LE: u8 = 5;
pub const PCM_FLOAT: u8 = 6;

/* ── 音频设备 ── */

#[repr(C)]
pub struct AudioDevice {
    pub name: [u8; 32],
    pub vendor: [u8; 16],
    pub caps: AudioCaps,
    pub state: AudioState,
    pub current_format: AudioFormat,
    pub buffer: *mut u8,
    pub buffer_size: usize,
    pub ops: &'static AudioDeviceOps,
    pub priv_data: *mut (),
}

#[repr(C)]
pub struct AudioCaps {
    pub max_channels: u8,
    pub min_rate: u32,
    pub max_rate: u32,
    pub formats: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u8,
    pub format: u8,
    pub buffer_frames: u32,
}

#[repr(C)]
pub struct AudioDeviceOps {
    pub open: unsafe extern "C" fn(dev: *mut AudioDevice, fmt: *const AudioFormat) -> i32,
    pub close: unsafe extern "C" fn(dev: *mut AudioDevice),
    pub start: unsafe extern "C" fn(dev: *mut AudioDevice) -> i32,
    pub stop: unsafe extern "C" fn(dev: *mut AudioDevice),
    pub write: unsafe extern "C" fn(dev: *mut AudioDevice, buf: *const u8, frames: u32) -> i32,
    pub read: unsafe extern "C" fn(dev: *mut AudioDevice, buf: *mut u8, frames: u32) -> i32,
    pub set_volume: unsafe extern "C" fn(dev: *mut AudioDevice, vol: u8),
    pub get_position: unsafe extern "C" fn(dev: *mut AudioDevice) -> u64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum AudioState {
    Closed = 0,
    Open = 1,
    Running = 2,
    Paused = 3,
}

/* ── 混音器 ── */

#[derive(Clone, Copy)]
pub struct MixerCtl {
    pub name: [u8; 32],
    pub ctl_type: MixerCtlType,
    pub min_val: i32,
    pub max_val: i32,
    pub value: i32,
}

#[derive(Clone, Copy)]
pub enum MixerCtlType {
    Volume = 0,
    Mute = 1,
    Source = 2,
    Gain = 3,
}

pub struct AudioMixer {
    pub controls: [MixerCtl; 64],
    pub control_count: usize,
    pub master_volume: u8,
}

/* ── 全局管理 ── */

pub struct AudioManager {
    pub devices: *mut AudioDevice,
    pub device_count: usize,
    pub mixer: AudioMixer,
    pub initialized: bool,
}

unsafe impl Send for AudioManager {}
static AUDIO_MANAGER: SpinLock<AudioManager> = SpinLock::new(AudioManager {
    devices: core::ptr::null_mut(),
    device_count: 0,
    mixer: AudioMixer { controls: [
        MixerCtl { name: [0; 32], ctl_type: MixerCtlType::Volume, min_val: 0, max_val: 0, value: 0 }; 64
    ], control_count: 0, master_volume: 75 },
    initialized: false,
});

pub fn init() {
    crate::serial::write_str(b"audio: initializing audio subsystem...\n");
    let mut mgr = AUDIO_MANAGER.lock();
    mgr.initialized = true;
    crate::serial::write_str(b"audio: HDA probe...\n");
    hda::probe();
    crate::serial::write_str(b"audio: AC97 probe...\n");
    ac97::probe();
    crate::serial::write_str(b"audio: subsystem ready\n");
}

pub fn register_device(dev: &'static mut AudioDevice) -> i32 {
    let mut mgr = AUDIO_MANAGER.lock();
    dev.state = AudioState::Closed;
    mgr.device_count += 1;
    crate::serial::write_str(b"audio: registered device\n");
    0
}
