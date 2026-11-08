package io.palm.client

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.util.AttributeSet
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View

/** 触控板手势区（§5.1）：touch-action:none 的对应物 = 全接管 onTouchEvent */
class TouchpadView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    var engine: GestureEngine? = null

    private val hintPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = 0x558B93A3.toInt()
        textSize = 34f
        textAlign = Paint.Align.CENTER
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (e.actionMasked == MotionEvent.ACTION_UP) performClick() // 无障碍要求
        return engine?.onTouch(e, this) ?: super.onTouchEvent(e)
    }

    override fun performClick(): Boolean = super.performClick()

    override fun onDraw(c: Canvas) {
        super.onDraw(c)
        c.drawText("触控板", width / 2f, height / 2f - 16, hintPaint)
        c.drawText("单指点按 · 双指滚动 · 长按拖拽", width / 2f, height / 2f + 28, hintPaint)
    }

    /** 长按拖拽触觉反馈（§5.2 navigator.vibrate 对应物） */
    fun haptic() = performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
}
