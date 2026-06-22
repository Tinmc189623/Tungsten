#!/usr/bin/env ruby
# build_tungsten.rb -- 编译 Tungsten 内核 (Rust)
# Copyright (C) 2026 Nexsteaduser. All rights reserved.
# SPDX-License-Identifier: GPL-3.0-or-later

require 'fileutils'

ROOT         = File.join(__dir__, '..')
TUNGSTEN_DIR = File.join(ROOT, 'src', 'tungsten')
BUILD_DIR    = File.join(ROOT, 'build')
KERNEL_ELF   = File.join(TUNGSTEN_DIR, 'target', 'x86_64-unknown-none', 'release', 'tungsten')
KERNEL_OUT   = File.join(BUILD_DIR, 'tungsten')

puts "==> 编译 Tungsten 内核..."
Dir.chdir(TUNGSTEN_DIR) do
  system('cargo', 'build',
         '--target', 'x86_64-unknown-none',
         '--release',
         '-Z', 'build-std=core,alloc') || abort("FAIL: 内核编译失败")
end

abort("FAIL: 未找到内核 ELF: #{KERNEL_ELF}") unless File.exist?(KERNEL_ELF)

FileUtils.mkdir_p(BUILD_DIR)
FileUtils.cp(KERNEL_ELF, KERNEL_OUT)
puts "==> 内核 ELF: #{KERNEL_OUT} (#{File.size(KERNEL_OUT)} bytes)"
