#!/bin/bash
# 把 Sotto 装成一个可以被 Dock 识别的 .app，然后（可选）放进 Dock。
#
#   ./scripts/install-app.sh          # 只生成 ~/Applications/Sotto.app
#   ./scripts/install-app.sh --dock   # 顺带加进 Dock
#
# 为什么需要它：Dock 只认 bundle。直接双击一个裸二进制是没有图标、也没有名字的。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$HOME/Applications/Sotto.app"
BIN="$ROOT/target/release/sotto-desktop"
ICNS="$ROOT/apps/desktop/icons/icon.icns"

if [ ! -x "$BIN" ]; then
  echo "还没编译。先跑：cargo build --release -p sotto-desktop" >&2
  exit 1
fi
if [ ! -f "$ICNS" ]; then
  echo "缺图标：$ICNS" >&2
  exit 1
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ICNS" "$APP/Contents/Resources/Sotto.icns"

cat > "$APP/Contents/MacOS/Sotto" <<'LAUNCHER'
#!/bin/bash
# 直接 exec GUI 二进制，这样 NSBundle 仍然解析成 Sotto.app，Dock 才显示对的图标
BIN="$HOME/Desktop/Sotto/target/release/sotto-desktop"
[ -x "$BIN" ] || BIN="$HOME/Desktop/Sotto/target/debug/sotto-desktop"
if [ ! -x "$BIN" ]; then
  osascript -e 'display alert "Sotto 还没编译" message "在 ~/Desktop/Sotto 执行：cargo build --release -p sotto-desktop"'
  exit 1
fi
exec "$BIN"
LAUNCHER
chmod +x "$APP/Contents/MacOS/Sotto"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Sotto</string>
  <key>CFBundleDisplayName</key><string>Sotto</string>
  <key>CFBundleIdentifier</key><string>com.zhuanz.sotto</string>
  <key>CFBundleExecutable</key><string>Sotto</string>
  <key>CFBundleIconFile</key><string>Sotto</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP"
echo "已生成：$APP"

if [ "${1:-}" = "--dock" ]; then
  defaults write com.apple.dock persistent-apps -array-add \
    "<dict><key>tile-data</key><dict><key>file-data</key><dict><key>_CFURLString</key><string>$APP</string><key>_CFURLStringType</key><integer>0</integer></dict><key>bundle-identifier</key><string>com.zhuanz.sotto</string><key>file-label</key><string>Sotto</string></dict><key>tile-type</key><string>file-tile</string></dict>"
  killall Dock 2>/dev/null || true
  echo "已加入 Dock"
fi
