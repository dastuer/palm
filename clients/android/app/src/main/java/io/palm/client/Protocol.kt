package io.palm.client

/**
 * Palm 二进制指针帧编码（§4.3，little-endian）。
 * 与 web/src/proto.ts、服务端 crates/palm-server/src/proto.rs 保持逐字节一致。
 */
object Protocol {
    const val GESTURE_UP = 0
    const val GESTURE_DOWN = 1
    const val GESTURE_LEFT = 2
    const val GESTURE_RIGHT = 3

    const val BTN_LEFT = 0
    const val BTN_RIGHT = 1
    const val BTN_MIDDLE = 2

    private const val MOVE = 0x01
    private const val SCROLL = 0x02
    private const val BUTTON = 0x03
    private const val TAP = 0x04
    private const val PINCH = 0x05
    private const val GESTURE = 0x06

    private fun i16(b: ByteArray, off: Int, v: Int) {
        val x = v.coerceIn(-32768, 32767)
        b[off] = (x and 0xff).toByte()
        b[off + 1] = ((x shr 8) and 0xff).toByte()
    }

    fun moveFrame(dx: Int, dy: Int): ByteArray {
        val b = ByteArray(5).also { it[0] = MOVE.toByte() }
        i16(b, 1, dx); i16(b, 3, dy)
        return b
    }

    fun scrollFrame(dx: Int, dy: Int): ByteArray {
        val b = ByteArray(5).also { it[0] = SCROLL.toByte() }
        i16(b, 1, dx); i16(b, 3, dy)
        return b
    }

    fun buttonFrame(btn: Int, down: Boolean): ByteArray =
        byteArrayOf(BUTTON.toByte(), btn.toByte(), if (down) 1 else 0)

    fun tapFrame(btn: Int, count: Int): ByteArray =
        byteArrayOf(TAP.toByte(), btn.toByte(), count.toByte())

    fun pinchFrame(dscale: Int, dx: Int, dy: Int): ByteArray {
        val b = ByteArray(7).also { it[0] = PINCH.toByte() }
        i16(b, 1, dscale); i16(b, 3, dx); i16(b, 5, dy)
        return b
    }

    fun gestureFrame(id: Int): ByteArray =
        byteArrayOf(GESTURE.toByte(), id.toByte())
}
