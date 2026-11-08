package io.palm.client

import android.os.Handler
import android.os.Looper
import android.view.Choreographer
import android.view.MotionEvent
import android.view.View
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.pow

/**
 * 手势状态机（§5.2），与 web/src/gesture.ts 逻辑一致：
 * 单指点按/移动/长按拖拽、双指滚动+惯性/右键/捏合、三指手势；
 * MOVE/SCROLL 发"位移增量"（px），灵敏度/加速在本层应用于 MOVE。
 */
class GestureEngine(
    private val send: (ByteArray) -> Unit,
    private val haptic: () -> Unit,
) {
    data class Opts(
        var sens: Float = 1.0f,
        var accelK: Float = 1.8f,       // 0 = 精确模式
        var scrollSpeed: Float = 1.0f,
        var tapEnabled: Boolean = true,
        var longPressMs: Long = 450L,
    )

    val opts = Opts()

    private enum class State { IDLE, PENDING, MOVE, DRAG, SCROLL, THREE, FREEZE }
    private var state = State.IDLE

    private class Pt(var x: Float, var y: Float)
    private val pts = HashMap<Int, Pt>()
    private var mainId = -1
    private var mainX = 0f; private var mainY = 0f
    private var downT = 0L
    private var accX = 0f; private var accY = 0f
    private var saccX = 0f; private var saccY = 0f
    private var pacc = 0f
    private var longPosted = false
    private val handler = Handler(Looper.getMainLooper())

    private var scrollIds = intArrayOf()
    private var cx = 0f; private var cy = 0f   // 双指中心
    private var twoDist = 0f
    private var twoStartT = 0L
    private var twoMoved = 0f
    private var pinchMode = false
    private class Sample(val t: Long, val x: Float, val y: Float)
    private val samples = ArrayDeque<Sample>()

    private var threeX = 0f; private var threeY = 0f
    private var threeFired = false

    private var lastTapEnd = 0L
    private var rafPosted = false
    private var lastRafT = 0L
    private var inertiaVX = 0f; private var inertiaVY = 0f
    private var inertiaLastT = 0L
    private var inertiaPosted = false

    // ---- 触摸入口（TouchpadView.onTouchEvent 转发）----

    fun onTouch(e: MotionEvent, view: View): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val idx = e.actionIndex
                val id = e.getPointerId(idx)
                pts[id] = Pt(e.getX(idx), e.getY(idx))
                onDown(id, view)
            }
            MotionEvent.ACTION_MOVE -> {
                for (i in 0 until e.pointerCount) {
                    val id = e.getPointerId(i)
                    pts[id]?.let {
                        applyMove(id, e.getX(i), e.getY(i))
                    }
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val id = e.getPointerId(e.actionIndex)
                pts.remove(id)
                onUp(id, cancel = false)
            }
            MotionEvent.ACTION_CANCEL -> {
                pts.clear()
                onAllCancelled(view)
            }
        }
        ensureRaf()
        return true
    }

    private fun center(list: Collection<Pt>): FloatArray {
        var x = 0f; var y = 0f
        for (p in list) { x += p.x; y += p.y }
        val n = list.size.coerceAtLeast(1)
        return floatArrayOf(x / n, y / n)
    }

    private fun onDown(id: Int, view: View) {
        val n = pts.size
        if (n > 3) return // 4+ 忽略（防误触）
        when (state) {
            State.IDLE -> {
                state = State.PENDING
                mainId = id
                pts[id]?.let { mainX = it.x; mainY = it.y }
                downT = System.currentTimeMillis()
                accX = 0f; accY = 0f
                postLongPress(view)
            }
            State.PENDING, State.MOVE -> {
                cancelLongPress()
                if (n >= 2) enterScroll()
            }
            State.SCROLL -> if (n >= 3) enterThree()
            else -> {}
        }
    }

    private fun postLongPress(view: View?) {
        longPosted = true
        handler.postDelayed({
            longPosted = false
            if (state == State.PENDING) enterDrag(view)
        }, opts.longPressMs)
    }

    private fun cancelLongPress() {
        if (longPosted) { handler.removeCallbacksAndMessages(null); longPosted = false }
    }

    private fun enterDrag(view: View?) {
        if (state != State.PENDING) return
        state = State.DRAG
        send(Protocol.buttonFrame(Protocol.BTN_LEFT, true))
        haptic() // Android 触觉反馈（§5.2）
        accX = 0f; accY = 0f
    }

    private fun enterScroll() {
        state = State.SCROLL
        accX = 0f; accY = 0f; saccX = 0f; saccY = 0f; pacc = 0f
        pinchMode = false
        scrollIds = pts.keys.take(2).toIntArray()
        val a = pts[scrollIds[0]] ?: return
        val b = pts[scrollIds.getOrNull(1)] ?: return
        cx = (a.x + b.x) / 2f; cy = (a.y + b.y) / 2f
        twoDist = hypot((a.x - b.x).toDouble(), (a.y - b.y).toDouble()).toFloat()
        twoStartT = System.currentTimeMillis()
        twoMoved = 0f
        samples.clear()
        samples.add(Sample(twoStartT, cx, cy))
    }

    private fun enterThree() {
        state = State.THREE
        val c = center(pts.values)
        threeX = c[0]; threeY = c[1]
        threeFired = false
    }

    private fun applyMove(id: Int, x: Float, y: Float) {
        val p = pts[id] ?: return
        val dx = x - p.x; val dy = y - p.y
        p.x = x; p.y = y
        when (state) {
            State.PENDING -> {
                accX += dx; accY += dy
                if (hypot((x - mainX).toDouble(), (y - mainY).toDouble()) > DEAD_ZONE_PX) {
                    cancelLongPress()
                    state = State.MOVE
                }
            }
            State.MOVE, State.DRAG -> { accX += dx; accY += dy }
            State.SCROLL -> {
                if (id !in scrollIds) return
                val a = pts[scrollIds[0]] ?: return
                val b = pts[scrollIds.getOrNull(1)] ?: return
                val ncx = (a.x + b.x) / 2f; val ncy = (a.y + b.y) / 2f
                val nd = hypot((a.x - b.x).toDouble(), (a.y - b.y).toDouble()).toFloat()
                twoMoved += hypot((ncx - cx).toDouble(), (ncy - cy).toDouble()).toFloat()
                if (!pinchMode && abs(nd - twoDist) > PINCH_ENTER_PX) pinchMode = true
                if (pinchMode) pacc += nd - twoDist
                else { saccX += ncx - cx; saccY += ncy - cy }
                cx = ncx; cy = ncy; twoDist = nd
                val now = System.currentTimeMillis()
                samples.addLast(Sample(now, ncx, ncy))
                while (samples.size > 2 && now - samples.first().t > 100) samples.removeFirst()
            }
            State.THREE -> {
                if (threeFired) return
                val c = center(pts.values)
                val dx3 = c[0] - threeX; val dy3 = c[1] - threeY
                if (hypot(dx3.toDouble(), dy3.toDouble()) > THREE_THRESH_PX) {
                    threeFired = true
                    send(
                        Protocol.gestureFrame(
                            if (abs(dx3) > abs(dy3)) {
                                if (dx3 > 0) Protocol.GESTURE_RIGHT else Protocol.GESTURE_LEFT
                            } else {
                                if (dy3 > 0) Protocol.GESTURE_DOWN else Protocol.GESTURE_UP
                            }
                        )
                    )
                }
            }
            else -> {}
        }
    }

    private fun onUp(id: Int, cancel: Boolean) {
        val now = System.currentTimeMillis()
        when (state) {
            State.PENDING -> {
                cancelLongPress()
                val main = pts[mainId]
                val dt = now - downT
                val moved = main?.let { hypot((it.x - mainX).toDouble(), (it.y - mainY).toDouble()) } ?: Double.MAX_VALUE
                if (!cancel && opts.tapEnabled && dt < TAP_MS && moved <= DEAD_ZONE_PX) {
                    val count = if (now - lastTapEnd < DBL_TAP_MS) 2 else 1
                    send(Protocol.tapFrame(Protocol.BTN_LEFT, count))
                    lastTapEnd = now
                }
                if (pts.size == 1) {
                    val nid = pts.keys.first()
                    val pt = pts[nid]!!
                    state = State.PENDING
                    mainId = nid
                    mainX = pt.x; mainY = pt.y
                    downT = now
                    postLongPress(null)
                } else {
                    state = State.IDLE
                }
            }
            State.MOVE -> {
                if (pts.size >= 2) enterScroll()
                else if (pts.isEmpty()) state = State.IDLE
            }
            State.DRAG -> {
                if (pts.isEmpty()) {
                    send(Protocol.buttonFrame(Protocol.BTN_LEFT, false))
                    state = State.IDLE
                }
            }
            State.SCROLL -> {
                if (pts.isEmpty()) {
                    if (!cancel && now - twoStartT < TAP_MS && twoMoved < DEAD_ZONE_PX) {
                        send(Protocol.tapFrame(Protocol.BTN_RIGHT, 1)) // 双指点按=右键
                    } else {
                        startInertia()
                    }
                    state = State.IDLE
                } else {
                    if (!cancel) startInertia()
                    state = State.FREEZE
                }
            }
            State.THREE, State.FREEZE -> if (pts.isEmpty()) state = State.IDLE
            else -> {}
        }
        ensureRaf()
    }

    private fun onAllCancelled(view: View?) {
        cancelLongPress()
        Choreographer.getInstance().removeFrameCallback(flushCb)
        rafPosted = false
        if (state == State.DRAG) send(Protocol.buttonFrame(Protocol.BTN_LEFT, false))
        state = State.IDLE
    }

    fun releaseAll() {
        onAllCancelled(null)
        handler.removeCallbacksAndMessages(null)
    }

    // ---- rAF 合帧（Choreographer，每帧每类最多 1 帧）----

    private val flushCb: Choreographer.FrameCallback = Choreographer.FrameCallback { t ->
        rafPosted = false
        val dt = ((t - lastRafT).coerceIn(1, 50)).toFloat()
        lastRafT = t
        when (state) {
            State.PENDING, State.MOVE, State.DRAG -> {
                if (accX != 0f || accY != 0f) {
                    val v = hypot(accX.toDouble(), accY.toDouble()).toFloat() / dt
                    val factor = 1f + opts.accelK * min(v / V_MAX_PXMS, 1f)
                    send(Protocol.moveFrame((accX * opts.sens * factor).toInt(), (accY * opts.sens * factor).toInt()))
                    accX = 0f; accY = 0f
                }
            }
            State.SCROLL -> {
                if (pinchMode) {
                    if (pacc != 0f) { send(Protocol.pinchFrame(pacc.toInt(), 0, 0)); pacc = 0f }
                } else if (saccX != 0f || saccY != 0f) {
                    send(Protocol.scrollFrame((saccX * opts.scrollSpeed).toInt(), (saccY * opts.scrollSpeed).toInt()))
                    saccX = 0f; saccY = 0f
                }
            }
            else -> {}
        }
        if (state in listOf(State.PENDING, State.MOVE, State.DRAG, State.SCROLL, State.THREE)) {
            rafPosted = true
            Choreographer.getInstance().postFrameCallback(flushCb)
        }
    }

    private fun ensureRaf() {
        if (rafPosted) return
        rafPosted = true
        lastRafT = System.nanoTime() / 1_000_000
        Choreographer.getInstance().postFrameCallback(flushCb)
    }

    // ---- 惯性（指数衰减）----

    private val inertiaCb: Choreographer.FrameCallback = Choreographer.FrameCallback { t ->
        inertiaPosted = false
        val dt = ((t - inertiaLastT).coerceIn(1, 50)).toFloat()
        inertiaLastT = t
        val dx = inertiaVX * dt
        val dy = inertiaVY * dt
        send(Protocol.scrollFrame(dx.toInt(), dy.toInt()))
        val decay = INERTIA_LAMBDA.pow(dt / 16.7f)
        inertiaVX *= decay; inertiaVY *= decay
        if (hypot(dx.toDouble(), dy.toDouble()) > INERTIA_STOP_PX &&
            hypot(inertiaVX.toDouble(), inertiaVY.toDouble()) > 0.02f
        ) {
            inertiaPosted = true
            Choreographer.getInstance().postFrameCallback(inertiaCb)
        }
    }

    private fun startInertia() {
        if (samples.size < 2) return
        val first = samples.first(); val last = samples.last()
        val dt = last.t - first.t
        if (dt < 5) return
        inertiaVX = (last.x - first.x) / dt * opts.scrollSpeed
        inertiaVY = (last.y - first.y) / dt * opts.scrollSpeed
        if (hypot(inertiaVX.toDouble(), inertiaVY.toDouble()) < 0.05) return
        inertiaLastT = System.nanoTime() / 1_000_000
        Choreographer.getInstance().removeFrameCallback(inertiaCb)
        inertiaPosted = true
        Choreographer.getInstance().postFrameCallback(inertiaCb)
    }

    companion object {
        private const val TAP_MS = 200L
        private const val DEAD_ZONE_PX = 8f
        private const val PINCH_ENTER_PX = 10f
        private const val THREE_THRESH_PX = 30f
        private const val DBL_TAP_MS = 300L
        private const val INERTIA_LAMBDA = 0.92f
        private const val INERTIA_STOP_PX = 2.0
        private const val V_MAX_PXMS = 2.0f
    }
}
