package io.palm.client

import android.annotation.SuppressLint
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Paint
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.Spanned
import android.text.TextWatcher
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import org.json.JSONObject

/**
 * Palm Android 主界面（对应 web/index.html）：顶栏状态 + 触控板/键盘/设置三面板 + 常驻键条。
 * 程序化构建 UI，避免大量 XML；协议见 docs/DESIGN.md §4。
 */
class MainActivity : AppCompatActivity(), PalmWs.Listener {

    private lateinit var store: SettingsStore
    private lateinit var ws: PalmWs
    private lateinit var engine: GestureEngine
    private lateinit var touchpad: TouchpadView
    private lateinit var touchTab: LinearLayout
    private lateinit var statusView: TextView
    private lateinit var rttView: TextView
    private lateinit var keysPanel: LinearLayout
    private lateinit var settingsPanel: ScrollView
    private lateinit var keybar: android.view.View
    private lateinit var imeInput: EditText
    private lateinit var pairBtn: Button
    private lateinit var unpairBtn: Button
    private val stickyMods = LinkedHashSet<String>()
    private val modButtons = mutableMapOf<String, Button>()
    private var addrDialog: AlertDialog? = null
    private var paired = false
    private var addrPrompted = false   // need_token 弹窗已自动弹过（防止重连循环反复弹出）
    private var lastState: String? = null

    // 应用内扫码：解析二维码里的 palm 链接（http://ip:port/#/t=token）或 ip:port 后直连
    private val scanLauncher = registerForActivityResult(ScanContract()) { result ->
        val contents = result.contents?.trim()
        if (contents.isNullOrEmpty()) return@registerForActivityResult
        if (contents.contains("t=")) {
            parsePalmUrl(contents)
        } else if (contents.contains(":")) {
            store.serverAddress = contents
        } else {
            Toast.makeText(this, "二维码里没有 Palm 地址", Toast.LENGTH_SHORT).show()
            return@registerForActivityResult
        }
        ws.close(); ws.connect()
    }

    // ---------- UI 构建 ----------

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    private fun darkButton(text: String, onClick: () -> Unit): Button {
        val b = Button(this)
        b.text = text
        b.setTextColor(0xFFE8EAF0.toInt())
        b.background = styles.bg(this)
        b.setPadding(dp(8), dp(12), dp(8), dp(12))
        b.setOnClickListener { onClick() }
        return b
    }

    /** 常驻键条用的小按钮（两行排布，不滚动） */
    private fun smallButton(text: String, onClick: () -> Unit): Button =
        darkButton(text, onClick).apply {
            textSize = 15f
            includeFontPadding = false
            setPadding(dp(2), dp(7), dp(2), dp(7))
            minWidth = 0; minHeight = 0
            minimumWidth = 0; minimumHeight = 0
        }

    private fun row(vararg views: android.view.View): LinearLayout {
        val l = LinearLayout(this)
        l.orientation = LinearLayout.HORIZONTAL
        for (v in views) {
            l.addView(v, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                margins(dp(3), 0, dp(3), 0)
            })
        }
        return l
    }

    @Suppress("DEPRECATION")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = SettingsStore(this)
        val metrics = resources.displayMetrics
        store.displayWidth = metrics.widthPixels
        store.displayHeight = metrics.heightPixels
        store.density = metrics.density

        // 二维码/分享链接直达：http://ip:port/#/t=token
        intent?.data?.toString()?.let { parsePalmUrl(it) }

        engine = GestureEngine(
            send = { ws.sendBinary(it) },
            haptic = { touchpad.haptic() },
        )
        applyEngineOpts()
        ws = PalmWs(store, this)

        val root = FrameLayout(this).apply { setBackgroundColor(0xFF0F1115.toInt()) }
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(0xFF0F1115.toInt())
        }
        root.addView(content, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        // 顶栏
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(12), dp(10), dp(12), dp(10))
            setBackgroundColor(0xFF161A21.toInt())
            gravity = Gravity.CENTER_VERTICAL
        }
        statusView = TextView(this).apply { setTextColor(0xFF8B93A3.toInt()); text = "—" }
        rttView = TextView(this).apply { setTextColor(0xFF8B93A3.toInt()); setPadding(dp(8), 0, 0, 0) }
        val tabTouch = TextView(this).apply { text = "触控"; setTextColor(0xFFE8EAF0.toInt()); setPadding(dp(12), dp(4), dp(12), dp(4)) }
        val tabKeys = TextView(this).apply { text = "键"; setTextColor(0xFF8B93A3.toInt()); setPadding(dp(12), dp(4), dp(12), dp(4)) }
        val tabCfg = TextView(this).apply { text = "⚙"; setTextColor(0xFF8B93A3.toInt()); setPadding(dp(12), dp(4), dp(12), dp(4)) }
        top.addView(statusView); top.addView(rttView)
        val spacer = android.view.View(this); top.addView(spacer, LinearLayout.LayoutParams(0, 1, 1f))
        top.addView(tabTouch); top.addView(tabKeys); top.addView(tabCfg)

        // 触控板页：触控板手势区 + 底部一体式左右键区域（按住=按下鼠标键，配合移动=拖拽）
        touchpad = TouchpadView(this)
        touchpad.engine = engine
        val clickZone = ClickZoneView(this).apply {
            press = { btn, down -> ws.sendBinary(Protocol.buttonFrame(btn, down)) }
        }
        touchTab = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            // 左右键区域高度 = 触控板的一半（2:1 分配）
            addView(touchpad, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 2f))
            addView(clickZone, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        }

        // 键盘面板
        keysPanel = buildKeysPanel()
        // 设置面板
        settingsPanel = buildSettingsPanel()

        val middle = FrameLayout(this).apply {
            addView(touchTab, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            addView(keysPanel, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
            addView(settingsPanel, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        }

        // 常驻键条（只在触控板页展示）
        keybar = buildKeybar()
        // 隐藏 IME 输入框（§5.1 compositionend 整句上屏的对应物）
        imeInput = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE
            layoutParams = LinearLayout.LayoutParams(dp(2), dp(2))
            alpha = 0.01f
        }
        imeInput.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
            override fun afterTextChanged(s: Editable) {
                val composing = s.getSpans(0, s.length, Any::class.java)
                    .any { (s.getSpanFlags(it) and Spanned.SPAN_COMPOSING) != 0 }
                if (!composing && s.isNotEmpty()) {
                    ws.send(JSONObject().put("t", "text").put("s", s.toString()))
                    s.clear()
                }
            }
        })
        imeInput.setOnEditorActionListener { _, _, _ ->
            flushIme(); ws.send(JSONObject().put("t", "tap").put("k", "enter")); true
        }

        content.addView(top)
        content.addView(middle, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        content.addView(keybar)
        content.addView(imeInput)

        setContentView(root)

        // targetSdk 35 在 Android 15+ 强制 edge-to-edge：给状态栏/导航栏/输入法让位
        ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
            val sb = insets.getInsets(WindowInsetsCompat.Type.statusBars())
            val nav = insets.getInsets(WindowInsetsCompat.Type.navigationBars())
            val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
            v.setPadding(0, sb.top, 0, maxOf(nav.bottom, ime.bottom))
            WindowInsetsCompat.CONSUMED
        }

        // 面板切换（键条只在触控板页显示）
        fun select(active: TextView, panel: android.view.View) {
            for (t in listOf(tabTouch, tabKeys, tabCfg)) t.setTextColor(0xFF8B93A3.toInt())
            active.setTextColor(0xFFE8EAF0.toInt())
            val onTouch = panel === touchTab
            touchTab.visibility = if (onTouch) android.view.View.VISIBLE else android.view.View.GONE
            keybar.visibility = if (onTouch) android.view.View.VISIBLE else android.view.View.GONE
            keysPanel.visibility = if (panel === keysPanel) android.view.View.VISIBLE else android.view.View.GONE
            settingsPanel.visibility = if (panel === settingsPanel) android.view.View.VISIBLE else android.view.View.GONE
        }
        tabTouch.setOnClickListener { select(tabTouch, touchTab) }
        tabKeys.setOnClickListener { select(tabKeys, keysPanel) }
        tabCfg.setOnClickListener { select(tabCfg, settingsPanel) }

        ws.connect()
    }

    override fun onDestroy() {
        super.onDestroy()
        engine.releaseAll()
        ws.close()
    }

    // ---------- 键盘面板 ----------

    @SuppressLint("SetTextI18n")
    private fun buildKeysPanel(): LinearLayout {
        val panel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(0xFF161A21.toInt())
            setPadding(dp(8), dp(8), dp(8), dp(12))
            visibility = android.view.View.GONE
        }
        // 媒体行
        panel.addView(row(
            darkButton("⏮") { sendMedia("prev") },
            darkButton("▶/⏸") { sendMedia("play") },
            darkButton("⏭") { sendMedia("next") },
            darkButton("🔉−") { sendMedia("voldown") },
            darkButton("🔇") { sendMedia("mute") },
            darkButton("🔊+") { sendMedia("volup") },
        ))
        // 功能键行
        panel.addView(row(
            darkButton("esc") { sendKey("esc") },
            darkButton("⇥") { sendKey("tab") },
            darkButton("⌫") { sendKey("backspace") },
            darkButton("⌦") { sendKey("delete") },
            darkButton("␣") { sendKey("space") },
            darkButton("↵") { sendKey("enter") },
        ))
        // F1–F12
        panel.addView(row(
            *Array(6) { i -> darkButton("F${i + 1}") { sendKey("f${i + 1}") } }
        ))
        panel.addView(row(
            *Array(6) { i -> darkButton("F${i + 7}") { sendKey("f${i + 7}") } }
        ))
        // 字母三行
        for (letters in listOf("qwertyuiop", "asdfghjkl", "zxcvbnm-/")) {
            panel.addView(row(*letters.map { ch -> darkButton(ch.toString()) { sendChar(ch) } }.toTypedArray()))
        }
        val tip = TextView(this).apply {
            text = "先点亮 ⌘⌥⌃⇧ 再按键 = 组合键；⌨️ 唤起输入法"
            setTextColor(0xFF8B93A3.toInt()); textSize = 12f
            gravity = Gravity.CENTER
        }
        panel.addView(tip)
        return panel
    }

    /** 常驻键条：修饰键一行 + 方向键/IME 一行，等宽铺满、不滚动 */
    private fun buildKeybar(): LinearLayout {
        val slot = {
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                margins(dp(3), dp(2), dp(3), dp(2))
            }
        }
        fun barRow(): LinearLayout = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
        }
        val modsRow = barRow()
        for ((name, label) in listOf("cmd" to "⌘", "opt" to "⌥", "ctrl" to "⌃", "shift" to "⇧")) {
            val b = smallButton(label) { toggleMod(name) }
            modButtons[name] = b
            modsRow.addView(b, slot())
        }
        val navRow = barRow()
        for ((key, label) in listOf("left" to "←", "up" to "↑", "down" to "↓", "right" to "→")) {
            navRow.addView(smallButton(label) { sendKey(key) }, slot())
        }
        val imeBtn = smallButton("⌨️") {
            imeInput.requestFocus()
            (getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager).showSoftInput(imeInput, InputMethodManager.SHOW_IMPLICIT)
        }
        navRow.addView(imeBtn, slot())
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(6), dp(2), dp(6), dp(6))
            setBackgroundColor(0xFF161A21.toInt())
        }
        bar.addView(modsRow, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        bar.addView(navRow, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        return bar
    }

    // ---------- 设置面板 ----------

    private fun buildSettingsPanel(): ScrollView {
        val body = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(10), dp(2), dp(10), dp(20))
        }
        fun sectionLabel(text: String): TextView = TextView(this).apply {
            this.text = text
            setTextColor(0xFF8B93A3.toInt())
            textSize = 12f
            setPadding(dp(4), dp(14), 0, dp(6))
        }
        fun card(vararg views: android.view.View): LinearLayout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = styles.bg(this@MainActivity)
            setPadding(dp(14), dp(4), dp(14), dp(6))
            for (v in views) addView(v)
        }
        fun slider(label: String, lo: Int, hi: Int, init: Int, fmt: (Int) -> String, onChange: (Int) -> Unit): LinearLayout {
            val value = TextView(this).apply {
                text = fmt(init)
                setTextColor(0xFF5B8CFF.toInt())
                textSize = 14f
            }
            val head = LinearLayout(this).apply { setPadding(0, dp(8), 0, dp(2)) }
            head.addView(TextView(this).apply { text = label; setTextColor(0xFFE8EAF0.toInt()); textSize = 15f },
                LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            head.addView(value)
            val sb = SeekBar(this).apply {
                max = hi - lo
                progress = init - lo
                progressTintList = ColorStateList.valueOf(0xFF5B8CFF.toInt())
                thumbTintList = ColorStateList.valueOf(0xFFE8EAF0.toInt())
                setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                    override fun onProgressChanged(s: SeekBar?, p: Int, fromUser: Boolean) {
                        value.text = fmt(p + lo)
                        onChange(p + lo)
                    }
                    override fun onStartTrackingTouch(s: SeekBar?) {}
                    override fun onStopTrackingTouch(s: SeekBar?) {}
                })
            }
            val l = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(0, 0, 0, dp(4)) }
            l.addView(head); l.addView(sb)
            return l
        }
        fun toggle(label: String, init: Boolean, onChange: (Boolean) -> Unit): LinearLayout {
            val sw = Switch(this).apply {
                isChecked = init
                thumbTintList = ColorStateList(
                    arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()),
                    intArrayOf(0xFFE8EAF0.toInt(), 0xFF6B7280.toInt()))
                trackTintList = ColorStateList(
                    arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()),
                    intArrayOf(0xFF5B8CFF.toInt(), 0xFF2A303B.toInt()))
                setOnCheckedChangeListener { _, c -> onChange(c) }
            }
            return LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                setPadding(0, dp(8), 0, dp(8))
                gravity = Gravity.CENTER_VERTICAL
                addView(TextView(this@MainActivity).apply { text = label; setTextColor(0xFFE8EAF0.toInt()); textSize = 15f },
                    LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
                addView(sw)
            }
        }
        body.addView(sectionLabel("指针"))
        body.addView(card(
            slider("灵敏度", 3, 30, (store.sens * 10).toInt(), { "${it / 10.0}x" }) { store.sens = it / 10f; applyEngineOpts(); pushCfg() },
            toggle("精确模式（关加速）", store.precise) { store.precise = it; applyEngineOpts() },
        ))
        body.addView(sectionLabel("滚动"))
        body.addView(card(
            slider("速度", 3, 30, (store.scrollSpeed * 10).toInt(), { "${it / 10.0}x" }) { store.scrollSpeed = it / 10f; applyEngineOpts(); pushCfg() },
            toggle("自然滚动（mac 方向）", store.natural) { store.natural = it; pushCfg() },
        ))
        body.addView(sectionLabel("手势"))
        body.addView(card(
            toggle("轻点（tap 点击）", store.tapEnabled) { store.tapEnabled = it; applyEngineOpts() },
            slider("长按拖拽阈值", 3, 12, store.longPressMs / 100, { "${it / 10.0}s" }) { store.longPressMs = it * 100; applyEngineOpts() },
        ))
        body.addView(sectionLabel("连接"))
        pairBtn = darkButton("配对电脑（扫码或输入地址）") { addrPrompted = false; showAddressDialog(null) }
        unpairBtn = darkButton("解除配对") {
            AlertDialog.Builder(this@MainActivity)
                .setTitle("解除配对")
                .setMessage("解除后需要重新扫码或输入地址才能连接电脑。确定解除？")
                .setPositiveButton("解除") { _, _ ->
                    // 只清本地凭证（服务端吊销走电脑端 /admin，协议与 web 端一致）；
                    // 无论电脑端在不在线都立刻给重新配对入口，在线时会收到 need_token 弹同一弹窗（已去重）
                    store.cred = null; store.token = null
                    ws.close()
                    addrPrompted = false
                    showAddressDialog(null)
                    ws.connect()
                }
                .setNegativeButton("取消", null)
                .show()
        }.apply { setTextColor(0xFFE5697E.toInt()) }
        body.addView(card(pairBtn, unpairBtn))
        updateConnectionUI()
        return ScrollView(this).apply {
            setBackgroundColor(0xFF161A21.toInt())
            isVerticalScrollBarEnabled = false
            visibility = android.view.View.GONE
            addView(body, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        }
    }

    /** 连接区按钮随配对状态切换：已连接显示"解除配对"，未连接显示"配对电脑" */
    private fun updateConnectionUI() {
        pairBtn.visibility = if (paired) android.view.View.GONE else android.view.View.VISIBLE
        unpairBtn.visibility = if (paired) android.view.View.VISIBLE else android.view.View.GONE
    }

    private fun applyEngineOpts() {
        engine.opts.sens = store.sens
        engine.opts.accelK = if (store.precise) 0f else 1.8f
        engine.opts.scrollSpeed = store.scrollSpeed
        engine.opts.tapEnabled = store.tapEnabled
        engine.opts.longPressMs = store.longPressMs.toLong()
    }

    private fun pushCfg() {
        ws.send(JSONObject().apply {
            put("t", "cfg")
            put("sens", store.sens.toDouble())
            put("natural", store.natural)
            put("scroll_speed", store.scrollSpeed.toDouble())
        })
    }

    // ---------- 按键发送 ----------

    private fun toggleMod(name: String) {
        if (!stickyMods.add(name)) stickyMods.remove(name)
        val on = name in stickyMods
        modButtons[name]?.alpha = if (on) 1f else 0.8f
        modButtons[name]?.background = styles.bg(this, if (on) 0xFF5B8CFF.toInt() else 0xFF1E232D.toInt())
    }

    private fun sendKey(k: String) {
        if (stickyMods.isNotEmpty()) {
            ws.send(JSONObject().put("t", "combo").put("ks", org.json.JSONArray(stickyMods.toList() + k)))
            clearSticky()
        } else {
            ws.send(JSONObject().put("t", "tap").put("k", k))
        }
    }

    private fun sendChar(c: Char) {
        if (stickyMods.isNotEmpty()) {
            ws.send(JSONObject().put("t", "combo").put("ks", org.json.JSONArray(stickyMods.toList() + c.toString())))
            clearSticky()
        } else {
            ws.send(JSONObject().put("t", "text").put("s", c.toString()))
        }
    }

    private fun sendMedia(k: String) = ws.send(JSONObject().put("t", "media").put("k", k))

    private fun clearSticky() {
        stickyMods.clear()
        for ((_, b) in modButtons) {
            b.alpha = 0.8f
            b.background = styles.bg(this)
        }
    }

    private fun flushIme() {
        val s = imeInput.text
        if (s.isNotEmpty()) {
            ws.send(JSONObject().put("t", "text").put("s", s.toString()))
            imeInput.setText("")
        }
    }

    // ---------- 配对/地址 ----------

    private fun parsePalmUrl(url: String) {
        val tok = Regex("t=([0-9a-fA-F]{8,64})").find(url)?.groupValues?.get(1)
        val host = Regex("https?://([^/:]+:\\d+)").find(url)?.groupValues?.get(1)
        if (tok != null) { store.token = tok; store.cred = null }
        if (host != null) store.serverAddress = host
    }

    private fun showAddressDialog(detail: String?) {
        if (addrDialog?.isShowing == true) return
        val input = EditText(this).apply {
            hint = "http://192.168.x.x:8787/#/t=… 或 ip:port"
            inputType = InputType.TYPE_CLASS_TEXT
            setText(store.serverAddress ?: "")
        }
        val dlg = AlertDialog.Builder(this)
            .setTitle("连接电脑")
            .setMessage(detail ?: "在电脑端运行 palm，扫码或粘贴完整链接")
            .setView(input)
            .setPositiveButton("连接") { _, _ ->
                val v = input.text.toString().trim()
                if (v.contains("t=")) {
                    parsePalmUrl(v)
                } else if (v.contains(":")) {
                    store.serverAddress = v
                }
                addrPrompted = false
                ws.close(); ws.connect()
            }
            .setNegativeButton("取消", null)
            .setNeutralButton("📷 扫码", null)
            .create()
        // 默认点中立键会关掉弹窗；拦截后只拉起扫码，返回时弹窗还在
        dlg.setOnShowListener {
            dlg.getButton(AlertDialog.BUTTON_NEUTRAL).setOnClickListener {
                scanLauncher.launch(
                    ScanOptions()
                        .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                        .setPrompt("对准电脑端 Palm 二维码")
                        .setBeepEnabled(false)
                        .setOrientationLocked(true)
                        .setCaptureActivity(PortraitScanActivity::class.java)
                )
            }
        }
        dlg.setOnDismissListener { if (addrDialog === dlg) addrDialog = null }
        addrDialog = dlg
        dlg.show()
    }

    private fun showPinDialog() {
        val input = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
            hint = "4 位 PIN"
        }
        AlertDialog.Builder(this)
            .setTitle("输入配对 PIN")
            .setView(input)
            .setPositiveButton("确认") { _, _ -> ws.submitPin(input.text.toString().trim()) }
            .setNegativeButton("取消", null)
            .show()
    }

    // ---------- PalmWs.Listener ----------

    override fun onState(state: String, detail: String?) {
        runOnUiThread {
            paired = state == "paired"
            updateConnectionUI()
            val dot = when (state) {
                "paired" -> "●"; "connecting", "pending" -> "◐"; else -> "○"
            }
            statusView.text = "$dot ${if (state == "paired") "已连接" else mapOf(
                "connecting" to "连接中", "pending" to "待配对", "offline" to "已断开",
                "busy" to "占用中", "need_token" to "未配对")[state] ?: state}"
            if (state != "paired") rttView.text = ""
            // 连接失败：状态变化时提示一次原因（PalmWs 只在每轮重连的首次失败上报，不会刷屏）
            if (state != lastState && state == "offline" && !detail.isNullOrEmpty()) {
                Toast.makeText(this, "连接失败：$detail", Toast.LENGTH_SHORT).show()
            }
            lastState = state
            // 未配对提示弹窗：一轮流程只自动弹一次，用户取消后不再反复弹出（可从设置页"配对电脑"手动打开）
            if (state == "need_token" && !addrPrompted) {
                addrPrompted = true
                showAddressDialog(detail)
            } else if (state == "busy" && !detail.isNullOrEmpty()) {
                Toast.makeText(this, detail, Toast.LENGTH_SHORT).show()
            }
        }
    }

    override fun onRtt(ms: Long) = runOnUiThread { rttView.text = "${ms}ms" }

    override fun onWelcome(cfg: JSONObject, trusted: Boolean) {
        runOnUiThread {
            (cfg.optDouble("sens"))?.let { store.sens = it.toFloat() }
            (cfg.optDouble("scroll_speed"))?.let { store.scrollSpeed = it.toFloat() }
            if (cfg.has("natural")) store.natural = cfg.optBoolean("natural")
            applyEngineOpts()
            if (!trusted) Toast.makeText(this, "电脑端缺少输入注入权限", Toast.LENGTH_LONG).show()
        }
    }

    override fun onPaired(name: String) {
        runOnUiThread { Toast.makeText(this, "已连接：$name", Toast.LENGTH_SHORT).show() }
    }

    override fun onPinNeeded() = runOnUiThread { showPinDialog() }

    override fun onToast(msg: String) = runOnUiThread {
        Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
    }
}

/** 触控板底部的左右键区域：按下=按下鼠标键，抬起=释放；配合触控板移动实现拖拽，轻点=单击 */
private class ClickZoneView(context: android.content.Context) : View(context) {
    var press: ((btn: Int, down: Boolean) -> Unit)? = null
    private var pressedBtn = -1
    private val divider = Paint().apply { color = 0xFF232A36.toInt(); strokeWidth = 2f; isAntiAlias = true }
    private val highlight = Paint().apply { color = 0x2A5B8CFF.toInt() }
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = 0xFF3E4654.toInt(); textSize = 26f; textAlign = Paint.Align.CENTER
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        val btn = if (e.x < width / 2f) Protocol.BTN_LEFT else Protocol.BTN_RIGHT
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> { pressedBtn = btn; press?.invoke(btn, true) }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (pressedBtn >= 0) press?.invoke(pressedBtn, false)
                pressedBtn = -1
                performClick()
            }
        }
        invalidate()
        return true
    }

    override fun performClick(): Boolean = super.performClick()

    override fun onDraw(c: Canvas) {
        super.onDraw(c)
        val half = width / 2f
        if (pressedBtn == Protocol.BTN_LEFT) c.drawRect(0f, 1f, half, height.toFloat(), highlight)
        if (pressedBtn == Protocol.BTN_RIGHT) c.drawRect(half, 1f, width.toFloat(), height.toFloat(), highlight)
        // 与触控板之间的水平分隔线 + 左右键之间的竖直分隔线
        c.drawLine(0f, 1f, width.toFloat(), 1f, divider)
        c.drawLine(half, 1f, half, height.toFloat(), divider)
        c.drawText("左键", half / 2, height / 2f + label.textSize / 3, label)
        c.drawText("右键", half + half / 2, height / 2f + label.textSize / 3, label)
    }
}

/** 背景样式的小工具（避免引 material 库） */
private object styles {
    fun bg(ctx: android.content.Context, color: Int = 0xFF1E232D.toInt()): android.graphics.drawable.GradientDrawable =
        android.graphics.drawable.GradientDrawable().apply {
            setColor(color)
            cornerRadius = 12f * ctx.resources.displayMetrics.density
        }
}

private fun ViewGroup.LayoutParams.margins(l: Int, t: Int, r: Int, b: Int) {
    (this as? ViewGroup.MarginLayoutParams)?.setMargins(l, t, r, b)
}
