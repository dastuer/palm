# Palm iOS 客户端

原生 iOS App（UIKit，程序化 UI），与电脑端 `palm-server` 通过同一 WS 协议通信。

## 构建

需要 [XcodeGen](https://github.com/yonaskolb/XcodeGen)（`brew install xcodegen`）：

```bash
cd clients/ios
xcodegen            # 生成 Palm.xcodeproj
open Palm.xcodeproj # Xcode 里选真机 → Run
```

> 免签名调试：Xcode → Signing & Capabilities 勾选 Automatically manage signing，用自己的 Apple ID（免费个人证书即可，7 天过期重签）。

## 结构

- `Protocol.swift` —— 二进制指针帧编码（§4.3），与 web/Android/服务端逐字节一致
- `PalmWs.swift` —— URLSessionWebSocketTask：连接/配对/重连/心跳 RTT
- `GestureEngine.swift` —— 手势状态机（§5.2），UIKit touches + CADisplayLink 合帧 + 惯性
- `ViewController.swift` —— 触控板/键盘/设置三面板、粘滞修饰键、IME 整句上屏（`markedTextRange` 判定组词）

## 注意

- iOS 上 WS 只能明文（ATS 对 `ws://` 需例外）：XcodeGen 的 info 属性里可加
  `NSAppTransportSecurity: { NSAllowsArbitraryLoads: true }`（局域网信任模型，与设计 §7 一致）
- Wake Lock 需要设置 `isIdleTimerDisabled = true`（可在 AppDelegate 里对 window 常开）
