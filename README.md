# Palm 🐾

> **P**hone-**A**s-**L**aptop-**M**ouse —— 把手机变成电脑的触控板。
> 手机浏览器扫码即用，电脑端单二进制，macOS 优先。

## 它能做什么

- 🖱️ **触控板**：单指移动/点按、双指滚动（带惯性）/右键、捏合缩放、长按拖拽、三指切换桌面/调度中心
- ⌨️ **键盘**：手机输入法整句上屏（原生支持中文 IME）、修饰键粘滞、组合键、媒体键
- 🔒 **安全**：二维码一次性 token 配对、可选 PIN、单控制会话、限流，零云端依赖
- ⚡ **低延迟**：二进制协议 + rAF 合帧 + 像素级滚动，局域网 RTT 级延迟

## 快速开始（macOS）

```bash
# 运行（首次会引导授予"辅助功能"权限——注入光标/键盘必需）
cargo run --release -p palm-server
# 或直接用构建产物
./target/release/palm
```

终端会打印二维码，手机相机扫码 → 浏览器打开即用（也可手动访问 `http://<电脑IP>:8787/#/t=<token>`）。

常用参数：

| 参数 | 说明 |
| --- | --- |
| `-p, --port N` | HTTP/WS 端口（默认 8787） |
| `--no-mdns` | 关闭 mDNS 广播 |
| `--web-dir DIR` | 开发模式：从磁盘目录提供前端 |
| `--config PATH` | 配置文件路径（默认 `~/.config/palm/palm.toml`） |

> 注入需要 **辅助功能** 权限：系统设置 → 隐私与安全性 → 辅助功能 → 勾选运行 palm 的终端 App。授权后无需重启，palm 会自动恢复注入。

## 手机端

- 首次：扫二维码自动配对（token 一次性）；配对成功后签发长期凭证存入手机 localStorage，之后直连免配对
- **触控**：单指点按/轻扫 · 双指滚动（抬手惯性）· 双指点按=右键 · 双指捏合=缩放 · 长按拖拽 · 三指上下左右=调度中心/APP 窗口/切换桌面
- **键盘**：底栏 ⌘⌥⌃⇧ 粘滞点亮后按任意键 = 组合键；⌨️ 唤起输入法整句上屏（支持中文 IME）；媒体键/F1-F12/方向键齐全
- **设置**：灵敏度、滚动速度、自然滚动、长按阈值、精确模式（关加速）、三指动作映射、深色模式
- 建议"添加到主屏幕"（PWA），防切后台断连 + 防息屏（Wake Lock）

## 构建

```bash
# 前端（产物内嵌进二进制）
cd web && npm install && npm run build && cd ..
# 服务端
cargo build --release          # 产物: target/release/palm
# 测试
cargo test                     # Rust 单元测试
node web/scripts/ws-test.mjs [port]   # WS 协议集成测试（需先启动服务端）
```

### macOS .app

```bash
./scripts/make-app.sh          # 产物 dist/Palm.app（Dock 常驻 + 菜单栏托盘）
```

安装后 `open /Applications/Palm.app`：

- **Dock 常驻爪印图标**，关闭窗口只是隐藏，点 Dock 图标或菜单栏图标再次打开；
- **主窗口「连接与配置」**（WKWebView 内嵌，首次启动自动弹出）：
  - 运行状态（版本 / 输入注入权限徽章 / mDNS）
  - 连接配对（二维码 + 链接复制 + 换新码）
  - 已配对设备列表与一键吊销
  - 配置（设备名 / 端口 / 配对 PIN），实时保存到 palm.toml
- **菜单栏托盘**：实时状态徽标（○ 等待接入 / ● 已连接 / ⚠ 权限缺失）+ 快捷入口 + 退出；
- 重复双击幂等：已有实例在跑时直接退出。

> 配对/连接功能全部在应用内完成；`http://127.0.0.1:8787/admin` 网页控制台仅作调试备用，App 不会主动打开浏览器。
>
> 打包脚本优先使用钥匙串中的 **"Palm Dev"** 自签名证书（`security find-identity -p codesigning` 可见）：
> 稳定签名让辅助功能授权在重装/更新后依然有效。首次授权后如系统提示，在
> 系统设置 → 隐私与安全性 → 辅助功能 中重新开关一次 Palm 即可。

### Windows / Linux 交叉检查与构建

```bash
# 编译验证（本机即可）
rustup target add x86_64-pc-windows-msvc x86_64-unknown-linux-gnu
cargo check --target x86_64-pc-windows-msvc      # SendInput 后端
cargo check --target x86_64-unknown-linux-gnu    # X11 XTest + uinput 后端
# 实际构建需在对应平台执行（Windows 装 MSVC Build Tools；Linux 需 pkg-config/libx11-dev 仅运行时链接）
```

平台说明：

| 平台 | 后端 | 备注 |
| --- | --- | --- |
| macOS | CGEvent | 需辅助功能权限，像素滚动/中文注入已实测 |
| Windows | SendInput | 自管虚拟光标 + ABSOLUTE\|VIRTUALDESK 防二次加速；高权限窗口（UAC）需管理员运行 |
| Linux X11 | XTEST（x11rb 纯 Rust） | 滚动为按钮事件量化（40px/格）；Unicode 文本依赖布局 keysym |
| Linux Wayland | /dev/uinput | 实验性；需 input 组权限，文本注入暂不支持 |

## 移动客户端（原生 App，协议与 PWA 完全一致）

| 客户端 | 目录 | 构建 |
| --- | --- | --- |
| Android（Kotlin） | [clients/android](clients/android) | Android Studio 打开或 `gradle assembleDebug`，详见 [README](clients/android/README.md) |
| iOS（Swift/UIKit） | [clients/ios](clients/ios) | `xcodegen && open Palm.xcodeproj`，详见 [README](clients/ios/README.md) |
| Web PWA（默认，零安装） | [web](web) | 扫码即用 |

三端共享同一套协议层与手势状态机（逐字节/逐状态对齐）：二进制指针帧 §4.3、配对流程 §4.4/§9.1、手势状态机 §5.2。

## 项目结构

```
palm/
├── crates/palm-server/        # 电脑端（Rust 单二进制）
│   └── src/
│       ├── main.rs            # CLI + 二维码 + TCP_NODELAY accept 循环
│       ├── http.rs            # axum：内嵌静态页 + /ws + /api/*
│       ├── ws.rs              # 会话状态机/心跳/单会话互斥/限流
│       ├── proto.rs           # JSON 控制帧 + 二进制指针帧
│       ├── security.rs        # 一次性 token/PIN/设备表/限流参数
│       ├── config.rs          # palm.toml 读写（权限 600）
│       ├── discovery.rs       # mDNS _palm._tcp
│       └── input/             # InputBackend trait + 平台后端
│           ├── macos.rs       # CGEvent（绝对移动/像素滚动/Unicode 注入/媒体键）
│           ├── windows.rs     # SendInput（ABSOLUTE|VIRTUALDESK/KEYEVENTF_UNICODE）
│           ├── linux.rs       # X11 XTEST + uinput 双后端，运行时选择
│           └── stub.rs        # 兜底占位
├── web/                       # 手机端（Vite+TS PWA，零框架）
│   └── src/                   # gesture/ws/keyboard/settings/proto
├── clients/
│   ├── android/               # 原生 Android 客户端（Kotlin）
│   └── ios/                   # 原生 iOS 客户端（Swift + XcodeGen）
└── scripts/make-app.sh        # macOS .app 打包
```

## 文档

- [设计文档](docs/DESIGN.md) —— 架构、协议、手势状态机、平台注入细节、里程碑
- [社区调研](docs/RESEARCH.md) —— Remote Mouse / AirMouse / Weylus / KDE Connect / 蓝牙 HID 等方案对比

## 状态

🚧 **开发中**。

| 里程碑 | 状态 | 说明 |
| --- | --- | --- |
| M0 骨架 | ✅ 完成 | Rust+axum、内嵌前端、WS 打通、macOS CGEvent 移动（实测光标注入生效） |
| M1 手势 | ✅ 完成 | 完整状态机、惯性滚动、右键/拖拽/捏合/三指、灵敏度/自然滚动（方向已实测校准） |
| M2 键盘 | ✅ 完成 | 修饰键粘滞、组合键、IME 整句注入、媒体键（本机实测） |
| M3 安全与稳定 | ✅ 完成 | token/PIN 配对、单会话互斥、限流断开、mDNS、断线重连、RTT HUD（集成测试 13/13） |
| M4 跨平台与分发 | ✅ 完成 | Windows SendInput / Linux XTEST+uinput 后端（交叉编译验证）；macOS .app（LSUIElement+icns）含**菜单栏 UI**（tray-icon+NSApp，状态实时刷新）与本机控制台页 /admin；Windows/Linux 上真机行为待回归 |
| 原生移动客户端 | ✅ 源码完成 | Android/iOS 工程与协议/手势完整移植，构建需 Android Studio / Xcode（本机无 SDK） |

已知限制：明文 WS（局域网信任模型，TLS 为可选进阶）、macOS Secure Input 密码框注入会被系统忽略、Linux Wayland 实验性（文本注入暂不支持）、X11 滚动按 40px 量化（见设计文档 §11/§13）。

## 技术栈

| 端 | 技术 |
| --- | --- |
| 电脑端 | Rust · axum/hyper · CGEvent · mDNS |
| 手机端 | Web PWA（TypeScript，零安装零框架） |
| 传输 | WebSocket（二进制指针帧 + JSON 控制帧，TCP_NODELAY） |
