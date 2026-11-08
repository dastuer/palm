// Palm iOS 协议层 —— 二进制指针帧编码（§4.3，little-endian）
// 与 web/src/proto.ts / 服务端 proto.rs / Android Protocol.kt 逐字节一致。

import Foundation

enum Protocol {
    static let gestureUp: UInt8 = 0
    static let gestureDown: UInt8 = 1
    static let gestureLeft: UInt8 = 2
    static let gestureRight: UInt8 = 3

    static let btnLeft: UInt8 = 0
    static let btnRight: UInt8 = 1
    static let btnMiddle: UInt8 = 2

    private static func putI16(_ b: inout Data, _ v: Int) {
        var x = Int16(clamping: v)
        withUnsafeBytes(of: &x) { b.append(contentsOf: $0) }
    }

    private static func frame(_ type: UInt8, cap: Int) -> Data {
        var d = Data(capacity: cap)
        d.append(type)
        return d
    }

    static func moveFrame(dx: Int, dy: Int) -> Data {
        var d = frame(0x01, cap: 5)
        putI16(&d, dx); putI16(&d, dy)
        return d
    }

    static func scrollFrame(dx: Int, dy: Int) -> Data {
        var d = frame(0x02, cap: 5)
        putI16(&d, dx); putI16(&d, dy)
        return d
    }

    static func buttonFrame(_ btn: UInt8, down: Bool) -> Data {
        var d = frame(0x03, cap: 3)
        d.append(btn)
        d.append(down ? 1 : 0)
        return d
    }

    static func tapFrame(_ btn: UInt8, count: Int) -> Data {
        var d = frame(0x04, cap: 3)
        d.append(btn)
        d.append(UInt8(clamping: count))
        return d
    }

    static func pinchFrame(dscale: Int, dx: Int, dy: Int) -> Data {
        var d = frame(0x05, cap: 7)
        putI16(&d, dscale); putI16(&d, dx); putI16(&d, dy)
        return d
    }

    static func gestureFrame(_ id: UInt8) -> Data {
        var d = frame(0x06, cap: 2)
        d.append(id)
        return d
    }
}
