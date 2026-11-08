# 社区实现调研：手机作为电脑触控板

> 调研时间：2026-11。目的：梳理现有方案的形态、技术路线与短板，为 Palm 的设计提供依据。

## 1. 方案形态总览

| 形态 | 代表 | 手机端 | 电脑端 | 优点 | 缺点 |
| --- | --- | --- | --- | --- | --- |
| 商业闭源 App | Remote Mouse、Unified Remote、WiFi Mouse、Monect | 装 App | 装服务端 | 功能全（触控板/键盘/遥控/语音），打磨久 | 闭源、广告/付费墙、隐私顾虑、不能自定义 |
| 网页版开源（WiFi） | **AirMouse**、mobile-touchpad、Weylus、Go-Web-Remote-Control | 浏览器，零安装 | 服务端 + Web | 免安装、跨 iOS/Android、可魔改 | 手感参差、安全普遍缺失、macOS 支持弱 |
| 原生 App + 自定协议 | KDE Connect / GSConnect / Valent | 装 App | 装服务端 | 系统级集成、生态大（不止触控板） | 两端都要装、重 |
| 蓝牙 HID（无服务端） | Blek、GhostBoard（开源参考） | 装 App | **什么都不用装** | 零电脑依赖、系统原生识别 | 需 Android 9+（`BluetoothHidDevice`），iOS 不可能；手势/回显能力受 HID 限制 |
| 屏幕+输入反向流 | scrcpy（方向相反）、Deskreen | — | — | 工程参考价值大 | 不是同类产品 |

## 2. 重点项目分析

### 2.1 AirMouse（cornradio/AirMouse，★~300，Python）
与我们目标最接近的项目：手机浏览器打开网页即用，单指移动/点击、双指滚动/右键、三指拖拽、键盘、语音输入、宏面板、手柄映射、陀螺仪"飞鼠"。

- **技术栈**：Python + Flask-SocketIO + pynput；自签 HTTPS（Chrome 只在 HTTPS 下才给陀螺仪数据）；托盘启动、二维码连接页。
- **亮点**：手势识别放在前端 JS（`touchpad.js`），后端只收 `move/click/scroll` 等简单事件；免安装体验好。
- **短板（我们要解决的）**：
  - macOS 支持是"补丁式"的：滚动方向要手改代码取反（pynput 的 line-scroll 在两个平台上语义不一致）、特殊键映射表要手动删改；iOS Safari 下移动"卡卡的"。
  - 无鉴权：同一局域网任何人打开页面即可控制电脑。
  - 依赖运行时 Python 环境，分发给普通用户难。
- 结论：验证了"网页版 + 后端注入"路线可行；工程质量是我们超越的空间。

### 2.2 mobile-touchpad（huoyijie/mobile-touchpad，Node.js）
教学式实现：hammer.js 识别手势 → socket.io 传输 → robotjs 注入。作者对延迟的分析很有价值：
- 实时性瓶颈在 WiFi 质量：2.4G 频段拥挤（蓝牙/无线鼠标干扰）、路由器转发路径；5G 频段或电脑自建热点（包不过路由器）体验最好。
- socket.io 需锁定 websocket-only 传输，避免轮询回退；建议二进制序列化减小包。
- 短板：hammer.js 的 threshold/interval 配置不完全贴合触控板手感；robotjs 年久失修；无 IME、无安全。

### 2.3 Weylus（H-M-H/Weylus，★~9.7k，Rust）
把平板变成电脑手写板/触摸屏，**架构是最值得抄的**：
- 浏览器端用 Pointer/Touch 事件捕获触控与笔（含压感/倾角）→ WebSocket（9001）→ 服务端注入（Linux 用 uinput）；
- 反向通道用 WebRTC 回传屏幕画面；
- 服务端 Rust（warp），前端 TS，静态资源打进二进制，单文件分发。
- **平台差异明显**：完整能力（多点触控、压感、窗口级捕获）仅 Linux；macOS/Windows 功能缩水；不支持屏幕软键盘输入。
- 启示：Rust 单二进制 + 内嵌前端是分发体验最好的形态；输入注入必须按平台各写后端，没有银弹。

### 2.4 KDE Connect（kdeconnect-kde / GSConnect / Valent）
虚拟触控板插件 `kdeconnect.mousepad.request`：包结构仿 JSON-RPC，字段含 `dx/dy/button/scroll/key` 等；TCP 1716，配对走证书交换 + TLS。需要两端装 App。Valent 的[协议文档](https://valent.andyholmes.ca/docs/protocol.html)是设计自定协议的好参考。它的"配对-信任设备"模型值得借鉴。

### 2.5 蓝牙 HID 路线（Blek / GhostBoard）
Android 9（API 28）起 `BluetoothHidDevice` 允许 App 注册为 HID 设备（SDP 记录 + `sendReport()`），电脑端把手机当真实蓝牙键鼠，**无需装任何电脑软件**。限制：部分 OEM 不支持、iOS 无对应 API、HID 报文表达不了"回显/高级手势/IME"。适合作为后续原生 Android App 的增强路线，不做首选。

### 2.6 商业产品（功能对齐清单）
Remote Mouse / Unified Remote 提供：触控板、键盘（含多语言）、媒体遥控、应用切换、音量、电源、语音输入、文件传输。我们的 MVP 对齐其触控板+键盘核心，遥控面板与语音进 backlog。

## 3. 关键技术结论（设计输入）

1. **客户端选 Web（PWA）**：零安装是最大的体验差异点；Pointer/Touch 事件足以实现完整手势引擎；iOS/Android 通吃。原生 App 只在蓝牙 HID/USB 阶段才需要。
2. **手势状态机放前端**：`tap/dbl-tap/两指tap=右键/双指滚动+惯性/长按拖拽/三指切换` 全部在浏览器里判定，网络上只传"语义化结果"（move 增量、scroll 增量、click），而不是原始触摸点。
3. **传输用原生 WebSocket**：文本帧传 JSON 控制/键入，二进制帧传高频指针事件；禁用 Nagle、合帧到 rAF、断线快速重连。延迟预算见设计文档 §10。
4. **输入注入按平台实现**：macOS CGEvent（需要辅助功能授权，滚动要用像素级 ScrollEvent）、Windows SendInput（相对移动会被系统加速度二次放大，需用绝对坐标 + 自管虚拟光标；UAC 窗口需管理员）、Linux X11 XTest / Wayland uinput。跨平台库（enigo/pynput）能起步，但滚动、Unicode、多屏等细节都要平台代码兜底。
5. **文本/IME 是现方案的集体盲区**：浏览器 `compositionend` 拿到整句中文 → 服务端 Unicode 注入（macOS `CGEventKeyboardSetUnicodeString`、Windows `KEYEVENTF_UNICODE`），绕开KeyCode 映射问题，天然支持所有语言。
6. **安全是开源小项目的通病**：必须做配对（PIN/一次性 token）、单控制会话、限流；mDNS 发现 + 二维码连接。
7. **分发形态学 Weylus**：单二进制内嵌前端 + 二维码 + 配置文件；macOS 做成免 Dock 的 .app。

## 4. 参考链接

- https://github.com/cornradio/AirMouse
- https://github.com/huoyijie/mobile-touchpad
- https://github.com/H-M-H/Weylus （含 issue #162 架构讨论）
- https://github.com/kde/kdeconnect-kde 、 https://valent.andyholmes.ca/docs/protocol.html
- https://github.com/ToxicOrca/ghostboard-android-bluetooth-mouse-keyboard
- https://play.google.com/store/apps/details?id=io.appground.blek
- https://github.com/enigo-rs/enigo （含 Permissions.md，macOS 辅助功能权限说明）
- https://www.remotemouse.net 、https://www.unifiedremote.com
- https://github.com/hammerjs/hammer.js 、https://github.com/octalmage/robotjs
- https://socket.io/docs/v4/performance-tuning/
