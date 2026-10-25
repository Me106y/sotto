#!/bin/bash
# Dock 图标启动的就是这个脚本。
cd "$HOME/Desktop/Sotto" || exit 1
printf '\033]0;Sotto\007'
clear
exec ./target/release/sotto chat \
  --root "$HOME/Desktop/Note" \
  --include 项目经历 --include 面试准备 --include 学习 \
  --k 3
