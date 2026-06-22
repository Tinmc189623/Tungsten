#!/usr/bin/env ruby
# install_deps.rb -- Tungsten 内核构建依赖检查
# Copyright (C) 2026 Nexsteaduser. All rights reserved.
# SPDX-License-Identifier: GPL-3.0-or-later

def cmd?(name)
  system("where #{name} >nul 2>&1") || system("which #{name} >/dev/null 2>&1")
end

checks = {
  'Ruby'  => -> { cmd?('ruby') },
  'Zig'   => -> { cmd?('zig') },
  'Rust'  => -> { cmd?('rustc') && cmd?('cargo') },
  'Rustup'=> -> { cmd?('rustup') },
}

puts 'Tungsten 内核依赖检查'
all_ok = true
checks.each do |name, fn|
  ok = fn.call
  puts "  [#{ok ? ' OK ' : 'FAIL'}] #{name}"
  all_ok = false unless ok
end

if cmd?('rustup')
  targets = `rustup target list --installed 2>&1`
  has = targets.include?('x86_64-unknown-none')
  puts "  [#{has ? ' OK ' : 'FAIL'}] target x86_64-unknown-none"
  all_ok = false unless has
end

exit(all_ok ? 0 : 1)
