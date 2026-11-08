#!/bin/zsh
# 构建 Palm.app（LSUIElement 菜单栏外无 Dock 图标，§12 M4）
# 用法: ./scripts/make-app.sh   → 产物 dist/Palm.app
set -euo pipefail
cd "$(dirname "$0")/.."

APP=dist/Palm.app
BIN=target/release/palm

[[ -x $BIN ]] || { echo "先构建: cargo build --release"; exit 1; }
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# 图标：PNG → icns（需要 iconutil，macOS 自带）
if command -v iconutil >/dev/null && [[ -f web/public/icons/icon-512.png ]]; then
  ICONSET="$APP.iconset"
  mkdir -p "$ICONSET"
  cp web/public/icons/icon-512.png "$ICONSET/icon_512x512.png"
  cp web/public/icons/icon-192.png "$ICONSET/icon_128x128@2x.png"
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
  rm -rf "$ICONSET"
fi

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>              <string>Palm</string>
  <key>CFBundleDisplayName</key>       <string>Palm</string>
  <key>CFBundleIdentifier</key>        <string>io.palm.server</string>
  <key>CFBundleVersion</key>           <string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleExecutable</key>        <string>palm</string>
  <key>CFBundlePackageType</key>       <string>APPL</string>
  <key>CFBundleIconFile</key>          <string>AppIcon</string>
  <key>NSHighResolutionCapable</key>   <true/>
  <!-- 主窗口 WKWebView 加载本机控制台页（http://127.0.0.1）需要的 ATS 例外 -->
  <key>NSAppTransportSecurity</key>
  <dict>
    <key>NSAllowsArbitraryLoads</key>
    <true/>
  </dict>
  <key>LSMinimumSystemVersion</key>    <string>11.0</string>
  <!-- 启动时自动打开本机控制台页（/admin），给用户可见反馈 -->
  <key>LSEnvironment</key>
  <dict>
    <key>PALM_OPEN_ADMIN</key>
    <string>1</string>
  </dict>
</dict>
</plist>
PLIST

cp "$BIN" "$APP/Contents/MacOS/palm"
# 优先用稳定的 "Palm Dev" 证书（TCC 辅助功能授权跨重装保持有效），否则 ad-hoc
IDENTITY=$(security find-identity -p codesigning 2>/dev/null | grep -i '"Palm Dev"' | head -1 | awk -F'"' '{print $2}')
if [[ -n "${IDENTITY:-}" ]]; then
  codesign --force --sign "$IDENTITY" "$APP"
else
  codesign --force --sign - "$APP"
fi

echo "✅ $APP"
echo "   运行: open $APP   （或直接 $APP/Contents/MacOS/palm）"
echo "   注入权限: 系统设置 → 隐私与安全性 → 辅助功能 → 勾选 Palm
   （Dock 图标常驻；关闭窗口只是隐藏，点 Dock 或菜单栏图标可再次打开）"
