// Palm iOS 主界面（对应 web/index.html）：顶栏 + 触控/键盘/设置三面板 + 常驻键条。
// 程序化 UIKit，无 storyboard。

import UIKit

final class ViewController: UIViewController, PalmWs.Listener, UITextFieldDelegate {
    private let store = SettingsStore.shared
    private var ws: PalmWs!
    private var engine: GestureEngine!

    private var statusLabel: UILabel!
    private var rttLabel: UILabel!
    private var touchpad: TouchpadView!
    private var keysPanel: UIView!
    private var settingsPanel: UIView!
    private var imeField: UITextField!
    private var tabs: [UIButton] = []
    private var modButtons: [String: UIButton] = [:]
    private var stickyMods: [String] = []

    private var themeBG = UIColor(red: 0.059, green: 0.067, blue: 0.082, alpha: 1)
    private var themePanel = UIColor(red: 0.086, green: 0.102, blue: 0.129, alpha: 1)
    private var themeBtn = UIColor(red: 0.118, green: 0.137, blue: 0.176, alpha: 1)
    private var themeFG = UIColor(red: 0.91, green: 0.92, blue: 0.94, alpha: 1)
    private var themeDim = UIColor(red: 0.55, green: 0.58, blue: 0.64, alpha: 1)
    private var themeAccent = UIColor(red: 0.36, green: 0.55, blue: 1.0, alpha: 1)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = themeBG
        engine = GestureEngine()
        ws = PalmWs()
        ws.listener = self

        buildTopbar()
        touchpad = TouchpadView(frame: .zero)
        keysPanel = buildKeysPanel()
        settingsPanel = buildSettingsPanel()
        buildKeybar()
        buildImeField()
        layoutAll()

        // 手势引擎接线：帧数据 → WS；触觉 → UIImpactFeedback
        engine.send = { [weak self] data in self?.ws.sendBinary(data) }
        engine.haptic = { UIImpactFeedbackGenerator(style: .medium).impactOccurred() }
        touchpad.engine = engine
        applyEngineOpts()
        ws.connect()
    }

    override var prefersStatusBarHidden: Bool { false }

    // ---- UI ----

    private func makeBtn(_ title: String, action: @escaping () -> Void) -> UIButton {
        let b = UIButton(type: .system)
        b.setTitle(title, for: .normal)
        b.setTitleColor(themeFG, for: .normal)
        b.backgroundColor = themeBtn
        b.layer.cornerRadius = 10
        b.titleLabel?.font = .systemFont(ofSize: 15, weight: .medium)
        b.addAction(UIAction { _ in action() }, for: .touchUpInside)
        return b
    }

    private func buildTopbar() {
        statusLabel = UILabel()
        statusLabel.text = "○ —"
        statusLabel.textColor = themeDim
        statusLabel.font = .systemFont(ofSize: 14)
        rttLabel = UILabel()
        rttLabel.textColor = themeDim
        rttLabel.font = .monospacedDigitSystemFont(ofSize: 14, weight: .regular)
        for (i, name) in ["触控", "键", "⚙"].enumerated() {
            let b = UIButton(type: .system)
            b.setTitle(name, for: .normal)
            b.setTitleColor(i == 0 ? themeFG : themeDim, for: .normal)
            b.tag = i
            b.addAction(UIAction { [weak self] _ in self?.selectTab(i) }, for: .touchUpInside)
            tabs.append(b)
        }
    }

    private func selectTab(_ idx: Int) {
        touchpad.isHidden = idx != 0
        keysPanel.isHidden = idx != 1
        settingsPanel.isHidden = idx != 2
        for (i, t) in tabs.enumerated() {
            t.setTitleColor(i == idx ? themeFG : themeDim, for: .normal)
        }
        if idx != 1 { imeField.resignFirstResponder() }
    }

    private func buildKeysPanel() -> UIView {
        let panel = UIView()
        panel.backgroundColor = themePanel
        panel.isHidden = true
        var rows: [UIStackView] = []
        func r(_ titles: [String], handler: @escaping (String) -> Void) -> UIStackView {
            let s = UIStackView(arrangedSubviews: titles.map { makeBtn($0) { handler($0) } })
            s.distribution = .fillEqually
            s.spacing = 6
            return s
        }
        rows.append(r(["⏮", "▶/⏸", "⏭", "🔉−", "🔇", "🔊+"]) { k in
            let media = ["⏮": "prev", "▶/⏸": "play", "⏭": "next", "🔉−": "voldown", "🔇": "mute", "🔊+": "volup"][k]!
            self.ws.send(["t": "media", "k": media])
        })
        rows.append(r(["esc", "⇥", "⌫", "⌦", "␣", "↵"]) { k in
            let key = ["esc": "esc", "⇥": "tab", "⌫": "backspace", "⌦": "delete", "␣": "space", "↵": "enter"][k]!
            self.sendKey(key)
        })
        rows.append(r((1...6).map { "F\($0)" }) { self.sendKey($0.lowercased()) })
        rows.append(r((7...12).map { "F\($0)" }) { self.sendKey($0.lowercased()) })
        rows.append(r(Array("qwertyuiop").map(String.init)) { self.sendChar($0.first!) })
        rows.append(r(Array("asdfghjkl").map(String.init)) { self.sendChar($0.first!) })
        rows.append(r(Array("zxcvbnm-/").map(String.init)) { self.sendChar($0.first!) })
        let stack = UIStackView(arrangedSubviews: rows)
        stack.axis = .vertical
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        panel.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: panel.topAnchor, constant: 8),
            stack.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 8),
            stack.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -8),
            stack.bottomAnchor.constraint(equalTo: panel.bottomAnchor, constant: -8),
        ])
        return panel
    }

    private func buildSettingsPanel() -> UIView {
        let panel = UIView()
        panel.backgroundColor = themePanel
        panel.isHidden = true
        let stack = UIStackView()
        stack.axis = .vertical
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        func slider(_ title: String, min: Double, max: Double, get: @escaping () -> Double, set: @escaping (Double) -> Void) -> UIStackView {
            let label = UILabel()
            label.text = "\(title)  \(String(format: "%.1fx", get()))"
            label.textColor = themeFG
            let s = UISlider()
            s.minimumValue = min; s.maximumValue = max
            s.value = get()
            s.addAction(UIAction { _ in
                set(Double(s.value))
                label.text = "\(title)  \(String(format: "%.1fx", get()))"
            }, for: .valueChanged)
            return UIStackView(arrangedSubviews: [label, s])
        }
        func toggle(_ title: String, get: @escaping () -> Bool, set: @escaping (Bool) -> Void) -> UIStackView {
            let label = UILabel(); label.text = title; label.textColor = themeFG
            let sw = UISwitch()
            sw.isOn = get()
            sw.addAction(UIAction { _ in set(sw.isOn) }, for: .valueChanged)
            let s = UIStackView(arrangedSubviews: [label, sw])
            s.axis = .horizontal
            return s
        }
        stack.addArrangedSubview(slider("指针灵敏度", min: 0.3, max: 3, get: { self.store.sens }, set: { self.store.sens = $0; self.applyEngineOpts(); self.pushCfg() }))
        stack.addArrangedSubview(slider("滚动速度", min: 0.3, max: 3, get: { self.store.scrollSpeed }, set: { self.store.scrollSpeed = $0; self.applyEngineOpts(); self.pushCfg() }))
        stack.addArrangedSubview(toggle("自然滚动（mac 方向）", get: { self.store.natural }, set: { self.store.natural = $0; self.pushCfg() }))
        stack.addArrangedSubview(toggle("轻点（tap 点击）", get: { self.store.tapEnabled }, set: { self.store.tapEnabled = $0; self.applyEngineOpts() }))
        stack.addArrangedSubview(toggle("精确模式（关加速）", get: { self.store.precise }, set: { self.store.precise = $0; self.applyEngineOpts() }))
        let unpair = makeBtn("解除配对（需在电脑端重新扫码）") {
            self.store.cred = nil; self.store.token = nil
            self.ws.close(); self.ws.connect()
        }
        unpair.setTitleColor(themeAccent, for: .normal)
        stack.addArrangedSubview(unpair)
        panel.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: panel.topAnchor, constant: 10),
            stack.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -14),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: panel.bottomAnchor, constant: -10),
        ])
        return panel
    }

    private func buildKeybar() {
        let bar = UIStackView()
        bar.axis = .horizontal
        bar.distribution = .fillEqually
        bar.spacing = 6
        for (name, label) in [("cmd", "⌘"), ("opt", "⌥"), ("ctrl", "⌃"), ("shift", "⇧"),
                              ("left", "←"), ("up", "↑"), ("down", "↓"), ("right", "→")] {
            if ["left", "up", "down", "right"].contains(name) {
                let b = makeBtn(label) { self.sendKey(name) }
                bar.addArrangedSubview(b)
            } else {
                let b = makeBtn(label) { self.toggleMod(name) }
                b.alpha = 0.8
                modButtons[name] = b
                bar.addArrangedSubview(b)
            }
        }
        let imeBtn = makeBtn("⌨️") { self.imeField.becomeFirstResponder() }
        bar.addArrangedSubview(imeBtn)
        bar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(bar)
        keybar = bar
    }

    private var keybar: UIStackView!

    private func buildImeField() {
        // IME 整句上屏（§5.1）：marked text = 组词中；无 marked 且非空 → 发送并清空
        imeField = UITextField()
        imeField.delegate = self
        imeField.autocorrectionType = .no
        imeField.autocapitalizationType = .none
        imeField.smartDashesType = .no
        imeField.smartQuotesType = .no
        imeField.spellCheckingType = .no
        imeField.returnKeyType = .send
        imeField.alpha = 0.01
        imeField.addTarget(self, action: #selector(imeChanged), for: .editingChanged)
    }

    @objc private func imeChanged() {
        if imeField.markedTextRange == nil, let s = imeField.text, !s.isEmpty {
            ws.send(["t": "text", "s": s])
            imeField.text = ""
        }
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        imeField.text = ""
        sendKey("enter")
        return true
    }

    private func layoutAll() {
        let top = UIStackView(arrangedSubviews: [statusLabel, rttLabel] + tabs)
        top.axis = .horizontal
        top.spacing = 10
        top.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(top)
        for p in [touchpad!, keysPanel, settingsPanel] {
            view.addSubview(p)
            p.translatesAutoresizingMaskIntoConstraints = false
        }
        NSLayoutConstraint.activate([
            top.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            top.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 14),
            top.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -14),

            touchpad.topAnchor.constraint(equalTo: top.bottomAnchor, constant: 4),
            touchpad.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            touchpad.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            touchpad.bottomAnchor.constraint(equalTo: keybar.topAnchor),

            keysPanel.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            keysPanel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            keysPanel.bottomAnchor.constraint(equalTo: keybar.topAnchor),
            keysPanel.topAnchor.constraint(greaterThanOrEqualTo: top.bottomAnchor),

            settingsPanel.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            settingsPanel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            settingsPanel.bottomAnchor.constraint(equalTo: keybar.topAnchor),
            settingsPanel.topAnchor.constraint(greaterThanOrEqualTo: top.bottomAnchor),

            keybar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 6),
            keybar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -6),
            keybar.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -6),
        ])
        view.addSubview(imeField)
        imeField.frame = CGRect(x: -50, y: -50, width: 10, height: 10)
    }

    // ---- 发送 ----

    private func applyEngineOpts() {
        engine.opts.sens = store.sens
        engine.opts.accelK = store.precise ? 0 : 1.8
        engine.opts.scrollSpeed = store.scrollSpeed
        engine.opts.tapEnabled = store.tapEnabled
        engine.opts.longPressMs = store.longPressMs
    }

    private func pushCfg() {
        ws.send(["t": "cfg", "sens": store.sens, "natural": store.natural, "scroll_speed": store.scrollSpeed])
    }

    private func toggleMod(_ name: String) {
        if let i = stickyMods.firstIndex(of: name) {
            stickyMods.remove(at: i)
        } else {
            stickyMods.append(name)
        }
        let on = stickyMods.contains(name)
        let b = modButtons[name]!
        b.alpha = on ? 1 : 0.8
        b.backgroundColor = on ? themeAccent : themeBtn
    }

    private func sendKey(_ k: String) {
        if !stickyMods.isEmpty {
            ws.send(["t": "combo", "ks": stickyMods + [k]])
            clearSticky()
        } else {
            ws.send(["t": "tap", "k": k])
        }
    }

    private func sendChar(_ c: Character) {
        if !stickyMods.isEmpty {
            ws.send(["t": "combo", "ks": stickyMods + [String(c)]])
            clearSticky()
        } else {
            ws.send(["t": "text", "s": String(c)])
        }
    }

    private func clearSticky() {
        stickyMods.removeAll()
        for (_, b) in modButtons {
            b.alpha = 0.8
            b.backgroundColor = themeBtn
        }
    }

    // ---- 配对 ----

    private func showAddressDialog(_ detail: String?) {
        let alert = UIAlertController(title: "连接电脑", message: detail ?? "在电脑端运行 palm，扫码或粘贴完整链接", preferredStyle: .alert)
        alert.addTextField {
            $0.placeholder = "http://192.168.x.x:8787/#/t=… 或 ip:port"
            $0.text = self.store.serverAddress
        }
        alert.addAction(UIAlertAction(title: "连接", style: .default) { _ in
            let v = alert.textFields?.first?.text?.trimmingCharacters(in: .whitespaces) ?? ""
            if let tok = v.range(of: #"t=([0-9a-fA-F]{8,64})"#, options: .regularExpression) {
                self.store.token = String(v[tok]).replacingOccurrences(of: "t=", with: "")
                self.store.cred = nil
            }
            if let host = v.range(of: #"https?://([^/:]+:\d+)"#, options: .regularExpression) {
                let u = String(v[host]).replacingOccurrences(of: "http://", with: "").replacingOccurrences(of: "https://", with: "")
                self.store.serverAddress = u
            } else if v.contains(":") {
                self.store.serverAddress = v
            }
            self.ws.close()
            self.ws.connect()
        })
        alert.addAction(UIAlertAction(title: "取消", style: .cancel))
        present(alert, animated: true)
    }

    private func showPinDialog() {
        let alert = UIAlertController(title: "输入配对 PIN", message: "电脑端 palm.toml [pairing] pin", preferredStyle: .alert)
        alert.addTextField { $0.keyboardType = .numberPad; $0.placeholder = "4 位 PIN" }
        alert.addAction(UIAlertAction(title: "确认", style: .default) { _ in
            self.ws.submitPin(alert.textFields?.first?.text?.trimmingCharacters(in: .whitespaces) ?? "")
        })
        alert.addAction(UIAlertAction(title: "取消", style: .cancel))
        present(alert, animated: true)
    }

    // ---- PalmWs.Listener ----

    func onState(_ state: String, detail: String?) {
        let dot: String
        let name: String
        switch state {
        case "paired": dot = "●"; name = "已连接"
        case "connecting", "pending": dot = "◐"; name = state == "pending" ? "待配对" : "连接中"
        case "offline": dot = "○"; name = "已断开"
        case "busy": dot = "○"; name = "占用中"
        default: dot = "○"; name = "未配对"
        }
        statusLabel.text = "\(dot) \(name)"
        if state != "paired" { rttLabel.text = "" }
        if state == "need_token" || state == "busy" {
            showAddressDialog(detail)
        }
    }

    func onRtt(_ ms: Double) {
        rttLabel.text = "\(Int(ms))ms"
    }

    func onWelcome(_ cfg: [String: Any], trusted: Bool) {
        if let s = cfg["sens"] as? Double { store.sens = s }
        if let s = cfg["scroll_speed"] as? Double { store.scrollSpeed = s }
        if let n = cfg["natural"] as? Bool { store.natural = n }
        applyEngineOpts()
        if !trusted {
            let alert = UIAlertController(title: "电脑端缺少输入注入权限", message: "在电脑端授予辅助功能权限后即可控制", preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "知道了", style: .default))
            present(alert, animated: true)
        }
    }

    func onPaired(_ name: String) {
        let alert = UIAlertController(title: "已连接：\(name)", message: nil, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "好", style: .default))
        present(alert, animated: true)
    }

    func onPinNeeded() { showPinDialog() }

    func onToast(_ msg: String) {
        let alert = UIAlertController(title: msg, message: nil, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "好", style: .default))
        present(alert, animated: true)
    }
}
