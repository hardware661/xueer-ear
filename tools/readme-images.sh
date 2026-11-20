#!/usr/bin/env bash
# 重新生成 README 配图（docs/images/）：先 tools/win.sh cargo build --release，再跑本脚本。
# 全是离屏渲染（不在屏幕上弹窗）：听写面板用 --panel-render，控制面板和翻译小窗用 --snapshot；
# 窗口截图补上 Windows 11 的圆角和阴影。需要 WSL 里的 ImageMagick 7（magick）。
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
build_win='D:\projects\_build\xueer-ear\target\release'
work_win='D:\projects\_build\ear-readme'
work=/mnt/d/projects/_build/ear-readme
out="$root/docs/images"
rm -rf "$work" && mkdir -p "$work/shots" "$out"
(cd /mnt/d/projects/_build && cmd.exe /c "$build_win\\xueer-ear-host.exe --panel-render $work_win clear" \
  && cmd.exe /c "$build_win\\xueer-ear.exe --snapshot $work_win\\shots")

# 窗口截图：圆角 12（1.5 倍缩放下的 8px）+ 柔和阴影，透明底
# （截图的透明通道是 GDI 留下的，不可信，先去掉）
window() {
  local size
  size=$(magick identify -format '%wx%h' "$1")
  magick "$1" -alpha off \
    \( -size "$size" xc:black -fill white -draw "roundrectangle 0,0 %[fx:w-1],%[fx:h-1] 12,12" \) \
    -compose CopyOpacity -composite \
    \( +clone -background black -shadow 28x14+0+8 \) +swap -background none -compose Over -layers merge +repage \
    -define png:color-type=6 "$2"
}

# 听写面板：说话中（3 倍渲染缩到 2 倍）
for theme in "" "-dark"; do
  magick "$work/panel-speaking$theme.png" -resize 66.67% "$out/panel$theme.png"
  magick "$work/panel-notice$theme.png" -resize 66.67% "$out/panel-notice$theme.png"
done
# 控制面板：浅色、深色并排
window "$work/shots/control-panel.png" "$work/cp-light.png"
window "$work/shots/dark/control-panel.png" "$work/cp-dark.png"
magick "$work/cp-light.png" "$work/cp-dark.png" -background none +smush -40 -define png:color-type=6 "$out/control-panel.png"
# 翻译小窗、托盘菜单：浅色、深色各一张
for theme in "" "dark/"; do
  suffix=${theme:+-dark}
  window "$work/shots/${theme}translate-window.png" "$out/translate$suffix.png"
  magick "$work/shots/${theme}tray-menu.png" -alpha off "$out/tray-menu$suffix.png"
done
ls -la "$out"
