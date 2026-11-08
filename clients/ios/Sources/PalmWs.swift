// Palm iOS WS 连接层（对应 web/src/ws.ts）：连接、配对、重连、心跳 RTT。
// 配对模型见设计 §4.4/§9.1：一次性 token → 可选 PIN → 长期 cred。

import Foundation
import UIKit

final class PalmWs: NSObject {
    protocol Listener: AnyObject {
        func onState(_ state: String, detail: String?)
        func onRtt(_ ms: Double)
        func onWelcome(_ cfg: [String: Any], trusted: Bool)
        func onPaired(_ name: String)
        func onPinNeeded()
        func onToast(_ msg: String)
    }

    private let store = SettingsStore.shared
    weak var listener: Listener?

    private var task: URLSessionWebSocketTask?
    private lazy var session: URLSession = URLSession(configuration: .default)
    private var retry = 0
    private var lastPingSent = 0.0
    private var pingTimer: Timer?
    private var closedByUs = false
    /// 连接代数：close/connect 递增，把旧 task 的在途回调全部作废（避免 close→connect 竞态导致双重连接）
    private var generation = 0
    /// 本次连接的配对尝试进度（0=cred 1=open 2=token 3=放弃），防回环
    private var pairStage = 0

    override init() {
        super.init()
        // 锁屏/切后台回来立即重连（§1.3）
        NotificationCenter.default.addObserver(
            forName: Notification.Name("palm.reconnect"), object: nil, queue: .main
        ) { [weak self] _ in
            if self?.task?.state != .running { self?.connect() }
        }
    }

    func connect() {
        generation += 1
        let gen = generation
        closedByUs = false
        guard let host = store.serverAddress else {
            listener?.onState("need_token", detail: "先填写电脑地址或扫码")
            return
        }
        guard let url = URL(string: "ws://\(host)/ws") else {
            listener?.onState("need_token", detail: "地址无效")
            return
        }
        listener?.onState("connecting", detail: nil)
        let t = session.webSocketTask(with: url)
        task = t
        t.resume()
        receive(t, gen: gen)
        sendHello()
    }

    private func sendHello() {
        // onOpen 无回调：发送 hello 成功即视为连接成功（失败走 receive 的 error）
        send([
            "t": "hello", "v": 1,
            "ua": "ios-\(UIDevice.current.model)",
            "w": Int(UIScreen.main.bounds.width),
            "h": Int(UIScreen.main.bounds.height),
            "dpr": UIScreen.main.scale,
            "touch": true,
            "device_id": SettingsStore.shared.deviceId,
        ])
    }

    private func receive(_ t: URLSessionWebSocketTask, gen: Int) {
        t.receive { [weak self] result in
            guard let self, gen == self.generation else { return }
            switch result {
            case .failure(let e):
                self.retryFailed(e, gen: gen)
            case .success(let msg):
                switch msg {
                case .string(let text):
                    self.handle(text)
                default:
                    break
                }
                self.receive(t, gen: gen)
            }
        }
    }

    private func retryFailed(_ e: Error, gen: Int) {
        DispatchQueue.main.async { [weak self] in
            guard let self, gen == self.generation, !self.closedByUs else { return }
            // 一轮重连只在首次失败时上报（带原因），避免循环刷屏
            if self.retry == 0 {
                self.listener?.onState("offline", detail: self.describe(e))
            }
            let delay = min(0.3 * pow(2.0, Double(self.retry)), 2.0)
            self.retry += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                if let self, gen == self.generation { self.connect() }
            }
        }
    }

    /// 连接失败原因的可读描述
    private func describe(_ e: Error) -> String {
        let e = e as NSError
        switch (e.domain, e.code) {
        case (NSURLErrorDomain, NSURLErrorCannotFindHost), (NSURLErrorDomain, NSURLErrorCannotConnectToHost):
            return "找不到电脑地址（palm 是否在运行）"
        case (NSURLErrorDomain, NSURLErrorTimedOut):
            return "连接超时"
        case (NSURLErrorDomain, NSURLErrorNetworkConnectionLost):
            return "网络连接中断"
        default:
            return e.localizedDescription
        }
    }

    private func handle(_ text: String) {
        guard let data = text.data(using: .utf8),
              let v = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            switch v["t"] as? String {
            case "welcome":
                self.retry = 0
                self.listener?.onState("pending", detail: nil)
                self.listener?.onWelcome(v["cfg"] as? [String: Any] ?? [:], trusted: v["trusted"] as? Bool ?? false)
                // 配对逐级回退：长期凭证（免配）→ 免配对码直连 → 一次性 token
                self.pairStage = 0
                self.attemptPair()
            case "pair_ok":
                self.retry = 0
                self.listener?.onState("paired", detail: nil)
                // 服务端签发/轮换的长期凭证：始终以服务端最新为准（§7）
                if let c = v["cred"] as? String, !c.isEmpty { self.store.cred = c }
                // 一次性 token 用完即清，避免下次误报"二维码已失效"
                self.store.token = nil
                self.listener?.onPaired(v["name"] as? String ?? "device")
                self.startPing()
            case "pair_need_pin":
                self.listener?.onPinNeeded()
            case "pair_err":
                switch v["e"] as? String {
                case "unknown_device":
                    self.store.cred = nil
                    self.attemptPair()
                case "open_disabled", "bad_mode":
                    self.pairStage = max(self.pairStage, 2)
                    self.attemptPair()
                case "bad_token":
                    self.store.token = nil
                    self.listener?.onToast("二维码已失效，请重新扫码")
                    self.listener?.onState("need_token", detail: "二维码已失效，请重新扫码")
                case "bad_pin":
                    self.listener?.onToast("PIN 错误，请重试")
                default:
                    let m = "配对失败：\(v["e"] as? String ?? "")"
                    self.listener?.onToast(m)
                    self.listener?.onState("need_token", detail: m)
                }
            case "pong":
                if self.lastPingSent > 0 {
                    self.listener?.onRtt(Date().timeIntervalSince1970 * 1000 - self.lastPingSent)
                }
            case "err":
                switch v["e"] as? String {
                case "busy":
                    self.listener?.onState("busy", detail: v["m"] as? String ?? "有设备正在控制")
                case "no_accessibility":
                    self.listener?.onToast("电脑端缺少辅助功能权限，无法注入输入")
                case "rate_limited":
                    self.listener?.onToast("发送过快被限流，请稍候")
                case "pair_timeout":
                    self.listener?.onToast("配对超时，请重新扫码")
                    self.listener?.onState("need_token", detail: "配对超时，请重新扫码")
                default:
                    break
                }
            default:
                break
            }
        }
    }

    private func startPing() {
        pingTimer?.invalidate()
        pingTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            guard let self, let task = self.task, task.state == .running else { return }
            self.lastPingSent = Date().timeIntervalSince1970 * 1000
            self.send(["t": "ping", "ts": self.lastPingSent])
        }
    }

    func send(_ obj: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: obj),
              let text = String(data: data, encoding: .utf8) else { return }
        task?.send(.string(text)) { _ in }
    }

    func sendBinary(_ data: Data) {
        task?.send(.data(data)) { _ in }
    }

    func submitPin(_ pin: String) {
        send(["t": "pair", "mode": "pin", "pin": pin])
    }

    /// 逐级配对：cred → open（免配对码直连）→ token，失败自动降级
    private func attemptPair() {
        if pairStage == 0, let cred = store.cred {
            pairStage = 1
            send(["t": "pair", "mode": "cred", "cred": cred])
            return
        }
        if pairStage <= 1 {
            pairStage = 2
            send(["t": "pair", "mode": "open"])
            return
        }
        if pairStage == 2, let token = store.token {
            pairStage = 3
            send(["t": "pair", "mode": "token", "token": token])
            return
        }
        listener?.onState("need_token", detail: "无法自动配对，请扫码")
    }

    func close() {
        closedByUs = true
        pingTimer?.invalidate()
        task?.cancel(with: .normalClosure, reason: nil)
        task = nil
    }
}
