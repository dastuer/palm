# Palm Android 客户端

原生 Android App，与电脑端 `palm-server` 通过同一 WS 协议通信（协议见 `docs/DESIGN.md` §4，
与 web PWA / iOS 客户端逐字节一致）。

## 构建

用 Android Studio 打开本目录（`clients/android/`），或命令行：

```bash
cd clients/android
gradle assembleDebug        # 产物: app/build/outputs/apk/debug/app-debug.apk
```

要求：JDK 17、Android SDK 35（Android Studio 会自动补齐 gradle wrapper）。

## 功能（对应 web/src 移植）

- `Protocol.kt` —— 二进制指针帧编码（§4.3）
- `PalmWs.kt` —— 连接/配对（token→PIN→cred）/指数退避重连/心跳 RTT（§4.4 §9.1）
- `GestureEngine.kt` —— 手势状态机（§5.2）：单指 tap/move/长按拖拽、双指滚动+惯性/右键/捏合、三指；MotionEvent + Choreographer 合帧
- `MainActivity.kt` —— 触控板/键盘/设置三面板 + 粘滞修饰键 + IME 整句上屏（SPAN_COMPOSING 判定）

## 使用

1. 电脑端运行 `palm`，App 内点状态栏输入电脑地址，或直接点开电脑端二维码里的链接（http://ip:port/#/t=…）
2. 首次自动配对并保存长期凭证；之后打开即连
