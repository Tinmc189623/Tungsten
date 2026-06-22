# Tungsten 内核顶层构建
# Copyright (C) 2026 Nexsteaduser. All rights reserved.
# SPDX-License-Identifier: GPL-3.0-or-later

.PHONY: all kernel hal clean deps

all: hal kernel

kernel:
	@echo "==> 编译 Tungsten 内核"
	ruby scripts/build_tungsten.rb

hal:
	@echo "==> 编译 Zig HAL"
	ruby scripts/build_hal.rb

deps:
	@echo "==> 检查构建依赖"
	ruby scripts/install_deps.rb --check

clean:
	@echo "==> 清理构建产物"
	ruby -e "require 'fileutils'; %w[src/tungsten/target src/hal/build build].each { |d| FileUtils.rm_rf(d) if Dir.exist?(d) }; Dir.glob('src/tungsten/*.{bin,rlib,a,o}').each { |f| File.delete(f) rescue nil }"
