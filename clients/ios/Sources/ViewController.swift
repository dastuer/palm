// Palm iOS 主界面（对应 web/index.html）：顶栏 + 触控/键盘/设置三面板 + 常驻键条。
// 程序化 UIKit，无 storyboard。
// 与 Android 客户端同步的交互：键条两行且只在触控页显示、触控板底部左右键区域（2:1）、
// 设置卡片化占满、配对/解除按钮随状态切换、解除需确认、连接失败 toast 提示原因、未配对弹窗只弹一次。

import UIKit

final class ViewController: UIViewController, PalmWs.Listener, UITextFieldDelegate {
    private let store = SettingsStore.shared
    private var ws: PalmWs!
    private var engine: GestureEngine!

    private var statusLabel: UILabel!
    private var rttLabel: UILabel!
    private var touchpad: TouchpadView!
    private var touchTab: UIView!
    private var clickZone: ClickZoneView!
    private var keysPanel: UIView!
    private var settingsPanel: UIView!
    private var imeField: UITextField!
    private var tabs: [UIButton] = []
    private var modButtons: [String: UIButton] = [:]
    private var stickyMods: [String] = []
    private var keybar: UIStackView!
    private var pairBtn: UIButton!
    private var unpairBtn: UIButton!
    private var toastView: UIView?

    private var paired = false
    private var addrPrompted = false  // need_token 弹窗已自动弹过（防止重连循环反复弹出）
    private var lastState: String?

    private var themeBG = UIColor(red: 0.059, green: 0.067, blue: 0.082, alpha: 1)
    private var themePanel = UIColor(red: 0.086, green: 0.102, blue: 0.129, alpha: 1)
    private var themeBtn = UIColor(red: 0.118, green: 0.137, blue: 0.176, alpha: 1)
    private var themeFG = UIColor(red: 0.91, green: 0.92, blue: 0.94, alpha: 1)
    private var themeDim = UIColor(red: 0.55, green: 0.58, blue: 0.64, alpha: 1)
    private var themeAccent = UIColor(red: 0.36, green: 0.55, blue: 1.0, alpha: 1)
    private var themeDanger = UIColor(red: 0.9, green: 0.41, blue: 0.49, alpha: 1)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = themeBG
        engine = GestureEngine()
        ws = PalmWs()
        ws.listener = self

        buildTopbar()
        touchTab = buildTouchTab()
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

    private func makeSmallBtn(_ title: String, action: @escaping () -> Void) -> UIButton {
        let b = makeBtn(title, action: action)
        b.titleLabel?.font = .systemFont(ofSize: 14, weight: .regular)
        b.layer.cornerRadius = 8
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
        touchTab.isHidden = idx != 0
        keysPanel.isHidden = idx != 1
        settingsPanel.isHidden = idx != 2
        keybar.isHidden = idx != 0  // 键条只在触控板页展示
        for (i, t) in tabs.enumerated() {
            t.setTitleColor(i == idx ? themeFG : themeDim, for: .normal)
        }
        if idx != 1 { imeField.resignFirstResponder() }
    }

    // 触控板页：手势区（上）+ 左右键区域（下，高度 = 触控板的一半）
    private func buildTouchTab() -> UIView {
        let tab = UIView()
        touchpad = TouchpadView(frame: .zero)
        clickZone = ClickZoneView(frame: .zero)
        clickZone.onPress = { [weak self] btn, down in
            self?.ws.sendBinary(Protocol.buttonFrame(btn, down: down))
        }
        let subviews: [UIView] = [touchpad, clickZone]
        for v in subviews {
            v.translatesAutoresizingMaskIntoConstraints = false
            tab.addSubview(v)
        }
        NSLayoutConstraint.activate([
            touchpad.topAnchor.constraint(equalTo: tab.topAnchor),
            touchpad.leadingAnchor.constraint(equalTo: tab.leadingAnchor),
            touchpad.trailingAnchor.constraint(equalTo: tab.trailingAnchor),
            clickZone.topAnchor.constraint(equalTo: touchpad.bottomAnchor),
            clickZone.leadingAnchor.constraint(equalTo: tab.leadingAnchor),
            clickZone.trailingAnchor.constraint(equalTo: tab.trailingAnchor),
            clickZone.bottomAnchor.constraint(equalTo: tab.bottomAnchor),
            clickZone.heightAnchor.constraint(equalTo: touchpad.heightAnchor, multiplier: 0.5),
        ])
        return tab
    }

    private func buildKeysPanel() -> UIView {
        let panel = UIView()
        panel.backgroundColor = themePanel
        panel.isHidden = true
        var rows: [UIStackView] = []
        func r(_ titles: [String], handler: @escaping (String) -> Void) -> UIStackView {
            let s = UIStackView(arrangedSubviews: titles.map { t in makeBtn(t, action: { handler(t) }) })
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

    // 设置页：卡片式分组（指针/滚动/手势/连接），整页占满可滚动
    private func buildSettingsPanel() -> UIView {
        let scroll = UIScrollView()
        scroll.backgroundColor = themePanel
        scroll.isHidden = true
        scroll.alwaysBounceVertical = true

        let stack = UIStackView()
        stack.axis = .vertical
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false

        func sectionLabel(_ text: String) -> UILabel {
            let l = UILabel()
            l.text = text
            l.textColor = themeDim
            l.font = .systemFont(ofSize: 12)
            return l
        }
        func card(_ views: [UIView]) -> UIView {
            let c = UIView()
            c.backgroundColor = themeBtn
            c.layer.cornerRadius = 12
            let s = UIStackView(arrangedSubviews: views)
            s.axis = .vertical
            s.spacing = 2
            s.translatesAutoresizingMaskIntoConstraints = false
            c.addSubview(s)
            NSLayoutConstraint.activate([
                s.topAnchor.constraint(equalTo: c.topAnchor, constant: 6),
                s.leadingAnchor.constraint(equalTo: c.leadingAnchor, constant: 14),
                s.trailingAnchor.constraint(equalTo: c.trailingAnchor, constant: -14),
                s.bottomAnchor.constraint(equalTo: c.bottomAnchor, constant: -8),
            ])
            return c
        }
        func slider(_ title: String, min: Double, max: Double, fmt: @escaping (Double) -> String,
                    get: @escaping () -> Double, set: @escaping (Double) -> Void) -> UIView {
            let label = UILabel()
            label.text = title
            label.textColor = themeFG
            label.font = .systemFont(ofSize: 15)
            let value = UILabel()
            value.text = fmt(get())
            value.textColor = themeAccent
            value.font = .monospacedDigitSystemFont(ofSize: 14, weight: .regular)
            value.setContentHuggingPriority(.required, for: .horizontal)
            let head = UIStackView(arrangedSubviews: [label, value])
            head.axis = .horizontal
            let s = UISlider()
            s.minimumValue = Float(min); s.maximumValue = Float(max)
            s.value = Float(get())
            s.minimumTrackTintColor = themeAccent
            s.maximumTrackTintColor = themeDim.withAlphaComponent(0.35)
            s.addAction(UIAction { _ in
                let v = Double(s.value)
                set(v)
                value.text = fmt(v)
            }, for: .valueChanged)
            let v0 = UIStackView(arrangedSubviews: [head, s])
            v0.axis = .vertical
            v0.spacing = 4
            let wrap = UIView()
            v0.translatesAutoresizingMaskIntoConstraints = false
            wrap.addSubview(v0)
            NSLayoutConstraint.activate([
                v0.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 6),
                v0.leadingAnchor.constraint(equalTo: wrap.leadingAnchor),
                v0.trailingAnchor.constraint(equalTo: wrap.trailingAnchor),
                v0.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -4),
            ])
            return wrap
        }
        func toggle(_ title: String, get: @escaping () -> Bool, set: @escaping (Bool) -> Void) -> UIView {
            let label = UILabel()
            label.text = title
            label.textColor = themeFG
            label.font = .systemFont(ofSize: 15)
            let sw = UISwitch()
            sw.isOn = get()
            sw.onTintColor = themeAccent
            sw.addAction(UIAction { _ in set(sw.isOn) }, for: .valueChanged)
            let s = UIStackView(arrangedSubviews: [label, sw])
            s.axis = .horizontal
            s.spacing = 8
            let wrap = UIView()
            s.translatesAutoresizingMaskIntoConstraints = false
            wrap.addSubview(s)
            NSLayoutConstraint.activate([
                s.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 8),
                s.leadingAnchor.constraint(equalTo: wrap.leadingAnchor),
                s.trailingAnchor.constraint(equalTo: wrap.trailingAnchor),
                s.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -8),
            ])
            return wrap
        }

        stack.addArrangedSubview(sectionLabel("指针"))
        stack.addArrangedSubview(card([
            slider("灵敏度", min: 0.3, max: 3, fmt: { String(format: "%.1fx", $0) },
                   get: { self.store.sens },
                   set: { self.store.sens = $0; self.applyEngineOpts(); self.pushCfg() }),
            toggle("精确模式（关加速）", get: { self.store.precise }, set: { self.store.precise = $0; self.applyEngineOpts() }),
        ]))
        stack.addArrangedSubview(sectionLabel("滚动"))
        stack.addArrangedSubview(card([
            slider("速度", min: 0.3, max: 3, fmt: { String(format: "%.1fx", $0) },
                   get: { self.store.scrollSpeed },
                   set: { self.store.scrollSpeed = $0; self.applyEngineOpts(); self.pushCfg() }),
            toggle("自然滚动（mac 方向）", get: { self.store.natural }, set: { self.store.natural = $0; self.pushCfg() }),
        ]))
        stack.addArrangedSubview(sectionLabel("手势"))
        stack.addArrangedSubview(card([
            toggle("轻点（tap 点击）", get: { self.store.tapEnabled }, set: { self.store.tapEnabled = $0; self.applyEngineOpts() }),
            slider("长按拖拽阈值", min: 0.3, max: 1.2, fmt: { String(format: "%.1fs", $0) },
                   get: { self.store.longPressMs / 1000 },
                   set: { self.store.longPressMs = $0 * 1000; self.applyEngineOpts() }),
        ]))
        stack.addArrangedSubview(sectionLabel("连接"))
        pairBtn = makeBtn("配对电脑（扫码或输入地址）") {
            self.addrPrompted = false
            self.showAddressDialog(nil)
        }
        unpairBtn = makeBtn("解除配对") {
            let confirm = UIAlertController(title: "解除配对", message: "解除后需要重新扫码或输入地址才能连接电脑。确定解除？", preferredStyle: .alert)
            confirm.addAction(UIAlertAction(title: "解除", style: .destructive) { _ in
                // 只清本地凭证（服务端吊销走电脑端 /admin，协议与 web 端一致）；
                // 无论电脑端在不在线都立刻给重新配对入口
                self.store.cred = nil
                self.store.token = nil
                self.ws.close()
                self.addrPrompted = false
                self.showAddressDialog(nil)
                self.ws.connect()
            })
            confirm.addAction(UIAlertAction(title: "取消", style: .cancel))
            self.present(confirm, animated: true)
        }
        unpairBtn.setTitleColor(themeDanger, for: .normal)
        stack.addArrangedSubview(card([pairBtn!, unpairBtn!]))
        updateConnectionUI()

        stack.setCustomSpacing(6, after: sectionLabel("指针"))
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 8),
            stack.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -12),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -16),
            stack.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -24),
        ])
        return scroll
    }

    /** 连接区按钮随配对状态切换：已连接显示"解除配对"，未连接显示"配对电脑" */
    private func updateConnectionUI() {
        pairBtn?.isHidden = paired
        unpairBtn?.isHidden = !paired
    }

    // 键条：修饰键一行 + 方向键/IME 一行，等宽铺满、不滚动；只在触控板页显示
    private func buildKeybar() {
        func row(_ builders: [(String, () -> UIButton)]) -> UIStackView {
            let s = UIStackView(arrangedSubviews: builders.map { $1() })
            s.axis = .horizontal
            s.distribution = .fillEqually
            s.spacing = 6
            return s
        }
        var mods: [(String, () -> UIButton)] = []
        for (name, label) in [("cmd", "⌘"), ("opt", "⌥"), ("ctrl", "⌃"), ("shift", "⇧")] {
            mods.append((label, { [weak self] in
                let b = self!.makeSmallBtn(label) { self!.toggleMod(name) }
                b.alpha = 0.8
                self?.modButtons[name] = b
                return b
            }))
        }
        var nav: [(String, () -> UIButton)] = []
        for (key, label) in [("left", "←"), ("up", "↑"), ("down", "↓"), ("right", "→")] {
            nav.append((label, { [weak self] in self!.makeSmallBtn(label) { self!.sendKey(key) } }))
        }
        nav.append(("⌨️", { [weak self] in self!.makeSmallBtn("⌨️") { self?.imeField.becomeFirstResponder() } }))
        keybar = UIStackView(arrangedSubviews: [row(mods), row(nav)])
        keybar.axis = .vertical
        keybar.spacing = 6
        keybar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(keybar)
    }

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
        let panels: [UIView] = [touchTab, keysPanel, settingsPanel]
        for p in panels {
            view.addSubview(p)
            p.translatesAutoresizingMaskIntoConstraints = false
        }
        NSLayoutConstraint.activate([
            top.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            top.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 14),
            top.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -14),

            // 触控板页占满 topbar 与键条之间
            touchTab.topAnchor.constraint(equalTo: top.bottomAnchor, constant: 4),
            touchTab.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            touchTab.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            touchTab.bottomAnchor.constraint(equalTo: keybar.topAnchor),

            keysPanel.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            keysPanel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            keysPanel.bottomAnchor.constraint(equalTo: keybar.topAnchor),
            keysPanel.topAnchor.constraint(greaterThanOrEqualTo: top.bottomAnchor),

            // 设置页占满
            settingsPanel.topAnchor.constraint(equalTo: top.bottomAnchor),
            settingsPanel.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            settingsPanel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            settingsPanel.bottomAnchor.constraint(equalTo: keybar.topAnchor),

            keybar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 6),
            keybar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -6),
            keybar.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -6),
        ])
        view.addSubview(imeField)
        imeField.frame = CGRect(x: -50, y: -50, width: 10, height: 10)
    }

    // ---- 轻提示（toast）----

    private func showToast(_ msg: String) {
        toastView?.removeFromSuperview()
        let c = UIView()
        c.backgroundColor = UIColor(white: 0.14, alpha: 0.94)
        c.layer.cornerRadius = 12
        let l = UILabel()
        l.text = msg
        l.textColor = themeFG
        l.font = .systemFont(ofSize: 14)
        l.numberOfLines = 0
        l.textAlignment = .center
        l.translatesAutoresizingMaskIntoConstraints = false
        c.addSubview(l)
        c.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(c)
        NSLayoutConstraint.activate([
            l.topAnchor.constraint(equalTo: c.topAnchor, constant: 10),
            l.bottomAnchor.constraint(equalTo: c.bottomAnchor, constant: -10),
            l.leadingAnchor.constraint(equalTo: c.leadingAnchor, constant: 16),
            l.trailingAnchor.constraint(equalTo: c.trailingAnchor, constant: -16),
            c.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            c.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -100),
        ])
        c.alpha = 0
        UIView.animate(withDuration: 0.2) { c.alpha = 1 }
        toastView = c
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.2) { [weak self, weak c] in
            guard let self, let c, self.toastView === c else { return }
            UIView.animate(withDuration: 0.25, animations: { c.alpha = 0 }) { _ in
                if self.toastView === c { self.toastView = nil }
                c.removeFromSuperview()
            }
        }
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

    private func parsePalmUrl(_ v: String) {
        if let tok = v.range(of: #"t=([0-9a-fA-F]{8,64})"#, options: .regularExpression) {
            store.token = String(v[tok]).replacingOccurrences(of: "t=", with: "")
            store.cred = nil
        }
        if let host = v.range(of: #"https?://([^/:]+:\d+)"#, options: .regularExpression) {
            let u = String(v[host]).replacingOccurrences(of: "http://", with: "").replacingOccurrences(of: "https://", with: "")
            store.serverAddress = u
        } else if v.contains(":") {
            store.serverAddress = v
        }
    }

    private func showAddressDialog(_ detail: String?) {
        let alert = UIAlertController(title: "连接电脑", message: detail ?? "在电脑端运行 palm，扫码或粘贴完整链接", preferredStyle: .alert)
        alert.addTextField {
            $0.placeholder = "http://192.168.x.x:8787/#/t=… 或 ip:port"
            $0.text = self.store.serverAddress
        }
        alert.addAction(UIAlertAction(title: "📷 扫码", style: .default) { _ in self.presentScanner() })
        alert.addAction(UIAlertAction(title: "连接", style: .default) { _ in
            let v = alert.textFields?.first?.text?.trimmingCharacters(in: .whitespaces) ?? ""
            self.parsePalmUrl(v)
            self.addrPrompted = false
            self.ws.close()
            self.ws.connect()
        })
        alert.addAction(UIAlertAction(title: "取消", style: .cancel))
        present(alert, animated: true)
    }

    private func presentScanner() {
        let s = ScannerViewController()
        s.onCode = { [weak self] code in
            guard let self else { return }
            self.parsePalmUrl(code)
            self.addrPrompted = false
            self.ws.close()
            self.ws.connect()
        }
        present(s, animated: true)
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
        paired = state == "paired"
        updateConnectionUI()
        // 连接失败：状态变化时提示一次原因（PalmWs 只在每轮重连的首次失败上报，不会刷屏）
        if state != lastState, state == "offline", let d = detail {
            showToast("连接失败：\(d)")
        }
        lastState = state
        // 未配对提示弹窗：一轮流程只自动弹一次，用户取消后不再反复弹出（可从设置页"配对电脑"手动打开）
        if state == "need_token", !addrPrompted {
            addrPrompted = true
            showAddressDialog(detail)
        } else if state == "busy", let d = detail {
            showToast(d)
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
        showToast("已连接：\(name)")
    }

    func onPinNeeded() { showPinDialog() }

    func onToast(_ msg: String) {
        showToast(msg)
    }
}
