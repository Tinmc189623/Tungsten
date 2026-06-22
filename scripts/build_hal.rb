#!/usr/bin/env ruby
# build_hal.rb -- 编译 Zig HAL 硬件抽象层
# Copyright (C) 2026 Nexsteaduser. All rights reserved.
# SPDX-License-Identifier: GPL-3.0-or-later

require 'fileutils'

ROOT      = File.join(__dir__, '..')
HAL_DIR   = File.join(ROOT, 'src', 'hal')
BUILD_DIR = File.join(ROOT, 'build')
HAL_BUILD = File.join(HAL_DIR, 'build')
HAL_LIB   = File.join(HAL_BUILD, 'libtungsten_hal.a')
HAL_OUT   = File.join(BUILD_DIR, 'libtungsten_hal.a')
ZIG       = ENV.fetch('ZIG', 'zig')

FileUtils.mkdir_p(HAL_BUILD)
FileUtils.mkdir_p(BUILD_DIR)

sources = Dir.glob(File.join(HAL_DIR, '*.zig')).map { |f| File.basename(f) }
abort("FAIL: 未找到 HAL Zig 源文件") if sources.empty?

puts "==> 编译 Zig HAL (#{sources.length} 个源文件)..."
object_files = []
sources.each do |src|
  src_path = File.join(HAL_DIR, src)
  obj_path = File.join(HAL_BUILD, src.sub('.zig', '.o'))
  print "  #{src}... "
  ok = system(ZIG, 'build-obj',
              '-target', 'x86_64-freestanding-none',
              '-O', 'ReleaseFast',
              '-fno-strip',
              '--name', src.sub('.zig', ''),
              '-femit-bin=' + obj_path,
              src_path)
  abort("FAIL: 编译 #{src} 失败") unless ok
  puts "OK"
  object_files << obj_path
end

FileUtils.rm_f(HAL_LIB)
abort("FAIL: 创建静态库失败") unless system(ZIG, 'ar', 'rcs', HAL_LIB, *object_files)
FileUtils.cp(HAL_LIB, HAL_OUT)
FileUtils.cp(HAL_LIB, File.join(ROOT, 'src', 'tungsten', 'libtungsten_hal.a'))
puts "==> HAL 静态库: #{HAL_OUT} (#{File.size(HAL_OUT)} bytes)"
