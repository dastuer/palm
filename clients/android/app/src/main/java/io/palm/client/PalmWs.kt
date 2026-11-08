package io.palm.client

import android.os.Handler
import android.os.Looper
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * WS 连接层（对应 web/src/ws.ts）：连接、配对流程、断线重连、心跳 RTT。
 * 配对模型见设计 §4.4/§9.1：一次性 token（URL 承载）→ 长期 cred。
 */
class PalmWs(
    private val store: SettingsStore,
    private val listener: Listener,
) {
    interface Listener {
        fun onState(state: String, detail: String? = null) // connecting/pending/paired/offline/busy/need_token
        fun onRtt(ms: Long)
        fun onWelcome(cfg: JSONObject, trusted: Boolean)
        fun onPaired(name: String)
        fun onPinNeeded()
        fun onToast(msg: String)
    }

    private val main = Handler(Looper.getMainLooper())
    private val client by lazy {
        OkHttpClient.Builder()
            .pingInterval(0, TimeUnit.SECONDS) // 心跳自己走 palm 协议
            .connectTimeout(4, TimeUnit.SECONDS)
            .build()
    }
    private var ws: WebSocket? = null
    private var retry = 0
    private var lastPingSent = 0L
    private var closedByUs = false
    /** 本次连接的配对尝试进度（0=cred 1=open 2=token 3=放弃），防回环 */
    private var pairStage = 0

    /** 连接代数：close/connect 递增，把旧 socket 的在途回调全部作废（避免 close→connect 竞态导致双重连接） */
    private var generation = 0L

    fun connect() {
        generation++
        val gen = generation
        ws?.cancel(); ws = null
        closedByUs = false
        val host = store.serverAddress ?: run { listener.onState("need_token", "先填写电脑地址或扫二维码"); return }
        val req = Request.Builder().url("ws://$host/ws").build()
        listener.onState("connecting")
        ws = client.newWebSocket(req, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                retry = 0
                val hello = JSONObject().apply {
                    put("t", "hello"); put("v", 1)
                    put("ua", "android-${android.os.Build.MODEL}")
                    put("w", store.displayWidth); put("h", store.displayHeight)
                    put("dpr", store.density); put("touch", true)
                    put("device_id", store.deviceId)
                }
                send(hello)
            }

            override fun onMessage(webSocket: WebSocket, text: String) = handle(text)

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                android.util.Log.w("PalmWs", "ws 失败", t)
                main.post {
                    if (gen != generation || closedByUs) return@post
                    if (retry == 0) { // 一轮重连只在首次失败时上报，避免循环刷屏
                        listener.onState("offline", describe(t))
                    }
                    scheduleReconnect(gen)
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                main.post {
                    if (gen != generation || closedByUs) return@post
                    if (retry == 0) listener.onState("offline", "电脑端已断开连接")
                    scheduleReconnect(gen)
                }
            }
        })
    }

    private fun scheduleReconnect(gen: Long) {
        val delay = (300L shl retry.coerceAtMost(4)).coerceAtMost(2000)
        retry++
        main.postDelayed({ if (gen == generation) connect() }, delay)
    }

    /** 逐级配对：cred → open（免配对码直连）→ token，失败自动降级 */
    private fun attemptPair() {
        val cred = store.cred
        when {
            pairStage == 0 && cred != null -> {
                pairStage = 1
                send(JSONObject().put("t", "pair").put("mode", "cred").put("cred", cred))
            }
            pairStage <= 1 -> {
                pairStage = 2
                send(JSONObject().put("t", "pair").put("mode", "open"))
            }
            pairStage == 2 && store.token != null -> {
                pairStage = 3
                send(JSONObject().put("t", "pair").put("mode", "token").put("token", store.token))
            }
            else -> listener.onState("need_token", "无法自动配对，请扫码")
        }
    }

    /** 连接失败原因的可读描述 */
    private fun describe(t: Throwable): String = when (t) {
        is java.net.UnknownHostException -> "找不到电脑地址"
        is java.net.ConnectException -> "电脑拒绝连接（palm 是否在运行）"
        is java.net.SocketTimeoutException -> "连接超时"
        is javax.net.ssl.SSLException -> "TLS 错误"
        else -> t.message?.take(80) ?: "网络错误"
    }

    private fun handle(text: String) {
        val v = JSONObject(text)
        when (v.optString("t")) {
            "welcome" -> {
                listener.onState("pending")
                listener.onWelcome(v.optJSONObject("cfg") ?: JSONObject(), v.optBoolean("trusted"))
                // 配对逐级回退：长期凭证（免配）→ 免配对码直连 → 一次性 token
                pairStage = 0
                attemptPair()
            }
            "pair_ok" -> {
                listener.onState("paired")
                // 服务端签发/轮换的长期凭证：始终以服务端最新为准（§7）
                v.optString("cred", "").takeIf { it.isNotEmpty() }?.let { store.cred = it }
                // 一次性 token 用完即清，避免下次误报"二维码已失效"
                store.token = null
                listener.onPaired(v.optString("name", "device"))
                startPing()
            }
            "pair_need_pin" -> listener.onPinNeeded()
            "pair_err" -> when (v.optString("e")) {
                "unknown_device" -> { store.cred = null; attemptPair() }
                "open_disabled", "bad_mode" -> { if (pairStage < 2) pairStage = 2; attemptPair() }
                "bad_token" -> {
                    store.token = null
                    listener.onState("need_token", "配对已失效，请刷新电脑端二维码后重扫")
                }
                "bad_pin" -> listener.onToast("PIN 错误，请重试")
                else -> {
                    val msg = "配对失败：" + v.optString("e")
                    listener.onToast(msg)
                    listener.onState("need_token", msg)
                }
            }
            "pong" -> listener.onRtt(System.currentTimeMillis() - lastPingSent)
            "err" -> when (v.optString("e")) {
                "busy" -> listener.onState("busy", v.optString("m", "有设备正在控制"))
                "no_accessibility" -> listener.onToast("电脑端缺少辅助功能权限，无法注入输入")
                "rate_limited" -> listener.onToast("发送过快被限流，请稍候")
                "pair_timeout" -> {
                    listener.onToast("配对超时，请重新扫码")
                    listener.onState("need_token", "配对超时，请重新扫码")
                }
            }
        }
    }

    private fun startPing() {
        val r = object : Runnable {
            override fun run() {
                ws?.let {
                    lastPingSent = System.currentTimeMillis()
                    send(JSONObject().put("t", "ping").put("ts", lastPingSent))
                    main.postDelayed(this, 5000)
                }
            }
        }
        main.post(r)
    }

    fun send(obj: JSONObject) {
        ws?.send(obj.toString())
    }

    fun sendBinary(bytes: ByteArray) {
        ws?.send(bytes.toByteString())
    }

    fun submitPin(pin: String) {
        send(JSONObject().put("t", "pair").put("mode", "pin").put("pin", pin))
    }

    fun close() {
        closedByUs = true
        generation++ // 作废在途回调，防止旧 socket 触发重连
        ws?.cancel()
        ws = null
    }
}
