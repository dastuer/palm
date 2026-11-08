// Palm iOS 手势状态机（§5.2，与 web/src/gesture.ts 逻辑一致）
// UIKit touches + CADisplayLink 合帧/惯性。

import UIKit

final class GestureEngine: NSObject {
    struct Opts {
        var sens: Double = 1.0
        var accelK: Double = 1.8       // 0 = 精确模式
        var scrollSpeed: Double = 1.0
        var tapEnabled: Bool = true
        var longPressMs: Double = 450
    }

    var opts = Opts()
    var send: (Data) -> Void = { _ in }
    var haptic: () -> Void = {}

    private enum State { case idle, pending, move, drag, scroll, three, freeze }
    private var state: State = .idle

    /// UITouch 按身份哈希（NSObject 语义），值为该触点当前位置
    private var pts: [UITouch: CGPoint] = [:]
    private var mainTouch: UITouch?
    private var mainAnchor: CGPoint = .zero
    private var downT: TimeInterval = 0
    private var acc = CGPoint.zero
    private var sacc = CGPoint.zero
    private var pacc: Double = 0
    private var longTimer: Timer?

    private var scrollTouches: [UITouch] = []
    private var twoCenter: CGPoint = .zero
    private var twoDist: CGFloat = 0
    private var twoStartT: TimeInterval = 0
    private var twoMoved: CGFloat = 0
    private var pinchMode = false
    private var samples: [(t: TimeInterval, x: CGFloat, y: CGFloat)] = []

    private var threeAnchor: CGPoint = .zero
    private var threeFired = false
    private var lastTapEnd: TimeInterval = 0

    private var displayLink: CADisplayLink?
    private var lastRafT: TimeInterval = 0
    private var inertiaLink: CADisplayLink?
    private var inertiaV = CGPoint.zero
    private var inertiaLastT: TimeInterval = 0

    private static let tapMs: TimeInterval = 0.2
    private static let deadZone: CGFloat = 8
    private static let pinchEnter: CGFloat = 10
    private static let threeThresh: CGFloat = 30
    private static let dblTapMs: TimeInterval = 0.3
    private static let inertiaLambda = 0.92
    private static let inertiaStopPx: CGFloat = 2
    private static let vMax: CGFloat = 2.0 // px/ms

    // ---- 触摸入口（TouchpadView 转发）----

    func touchesBegan(_ touches: Set<UITouch>) {
        for t in touches {
            pts[t] = t.location(in: t.view)
        }
        let n = pts.count
        guard n <= 3 else { return } // 4+ 忽略（防误触）
        switch state {
        case .idle:
            state = .pending
            let t = touches.first!
            mainTouch = t
            mainAnchor = pts[t]!
            downT = CACurrentMediaTime()
            acc = .zero
            scheduleLongPress()
        case .pending, .move:
            cancelLongPress()
            if n >= 2 { enterScroll() }
        case .scroll:
            if n >= 3 { enterThree() }
        default:
            break
        }
        ensureRaf()
    }

    func touchesMoved(_ touches: Set<UITouch>) {
        for t in touches {
            applyMove(t, t.location(in: t.view))
        }
        ensureRaf()
    }

    func touchesEnded(_ touches: Set<UITouch>, cancelled: Bool) {
        let now = CACurrentMediaTime()
        for t in touches {
            pts.removeValue(forKey: t)
        }
        switch state {
        case .pending:
            cancelLongPress()
            let dt = now - downT
            let main = mainTouch.flatMap { pts[$0] }
            let moved = main.map { hypot($0.x - mainAnchor.x, $0.y - mainAnchor.y) } ?? .greatestFiniteMagnitude
            if !cancelled && opts.tapEnabled && dt < Self.tapMs && moved <= Self.deadZone {
                let count = (now - lastTapEnd) < Self.dblTapMs ? 2 : 1
                send(Protocol.tapFrame(Protocol.btnLeft, count: count))
                lastTapEnd = now
            }
            if pts.count == 1, let (t, p) = pts.first {
                state = .pending
                mainTouch = t
                mainAnchor = p
                downT = now
                scheduleLongPress()
            } else {
                state = .idle
            }
        case .move:
            if pts.count >= 2 { enterScroll() }
            else if pts.isEmpty { state = .idle }
        case .drag:
            if pts.isEmpty {
                send(Protocol.buttonFrame(Protocol.btnLeft, down: false))
                state = .idle
            }
        case .scroll:
            if pts.isEmpty {
                if !cancelled && (now - twoStartT) < Self.tapMs && twoMoved < Self.deadZone {
                    send(Protocol.tapFrame(Protocol.btnRight, count: 1)) // 双指点按=右键
                } else {
                    startInertia()
                }
                state = .idle
            } else {
                if !cancelled { startInertia() }
                state = .freeze
            }
        case .three, .freeze:
            if pts.isEmpty { state = .idle }
        default:
            break
        }
        ensureRaf()
    }

    func releaseAll() {
        cancelLongPress()
        displayLink?.invalidate(); displayLink = nil
        inertiaLink?.invalidate(); inertiaLink = nil
        if state == .drag { send(Protocol.buttonFrame(Protocol.btnLeft, down: false)) }
        pts.removeAll()
        state = .idle
    }

    // ---- 内部 ----

    private func center(_ values: [CGPoint]) -> CGPoint {
        guard !values.isEmpty else { return .zero }
        let sx = values.reduce(0.0) { $0 + $1.x }
        let sy = values.reduce(0.0) { $0 + $1.y }
        return CGPoint(x: sx / CGFloat(values.count), y: sy / CGFloat(values.count))
    }

    private func scheduleLongPress() {
        longTimer = Timer.scheduledTimer(withTimeInterval: opts.longPressMs / 1000, repeats: false) { [weak self] _ in
            guard let self, self.state == .pending else { return }
            self.state = .drag
            self.send(Protocol.buttonFrame(Protocol.btnLeft, down: true))
            self.haptic()
            self.acc = .zero
        }
    }

    private func cancelLongPress() {
        longTimer?.invalidate()
        longTimer = nil
    }

    private func enterScroll() {
        state = .scroll
        acc = .zero; sacc = .zero; pacc = 0
        pinchMode = false
        scrollTouches = Array(pts.keys.prefix(2))
        let a = pts[scrollTouches[0]] ?? .zero
        let b = pts[scrollTouches[1]] ?? .zero
        twoCenter = CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
        twoDist = hypot(a.x - b.x, a.y - b.y)
        twoStartT = CACurrentMediaTime()
        twoMoved = 0
        samples = [(twoStartT, twoCenter.x, twoCenter.y)]
    }

    private func enterThree() {
        state = .three
        threeAnchor = center(Array(pts.values))
        threeFired = false
    }

    private func applyMove(_ t: UITouch, _ p: CGPoint) {
        guard let old = pts[t] else { return }
        let dx = p.x - old.x, dy = p.y - old.y
        pts[t] = p

        switch state {
        case .pending:
            acc.x += dx; acc.y += dy
            if hypot(p.x - mainAnchor.x, p.y - mainAnchor.y) > Self.deadZone {
                cancelLongPress()
                state = .move
            }
        case .move, .drag:
            acc.x += dx; acc.y += dy
        case .scroll:
            guard scrollTouches.contains(t) else { return }
            guard let other = scrollTouches.first(where: { $0 !== t }),
                  let op = pts[other] else { return }
            let a = p, b = op
            let nc = CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
            let nd = hypot(a.x - b.x, a.y - b.y)
            twoMoved += hypot(nc.x - twoCenter.x, nc.y - twoCenter.y)
            if !pinchMode && abs(nd - twoDist) > Self.pinchEnter { pinchMode = true }
            if pinchMode {
                pacc += Double(nd - twoDist)
            } else {
                sacc.x += nc.x - twoCenter.x
                sacc.y += nc.y - twoCenter.y
            }
            twoCenter = nc; twoDist = nd
            let now = CACurrentMediaTime()
            samples.append((now, nc.x, nc.y))
            while samples.count > 2 && now - samples[0].t > 0.1 { samples.removeFirst() }
        case .three:
            guard !threeFired else { return }
            let c = center(Array(pts.values))
            let dx3 = c.x - threeAnchor.x, dy3 = c.y - threeAnchor.y
            if hypot(dx3, dy3) > Self.threeThresh {
                threeFired = true
                let id: UInt8
                if abs(dx3) > abs(dy3) {
                    id = dx3 > 0 ? Protocol.gestureRight : Protocol.gestureLeft
                } else {
                    id = dy3 > 0 ? Protocol.gestureDown : Protocol.gestureUp
                }
                send(Protocol.gestureFrame(id))
            }
        default:
            break
        }
    }

    // ---- CADisplayLink 合帧（每帧每类最多 1 帧）----

    private func ensureRaf() {
        guard displayLink == nil else { return }
        lastRafT = CACurrentMediaTime()
        let link = CADisplayLink(target: self, selector: #selector(flush))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @objc private func flush() {
        let now = CACurrentMediaTime()
        let dtMs = min(50, max(1, (now - lastRafT) * 1000))
        lastRafT = now
        switch state {
        case .pending, .move, .drag:
            if acc != .zero {
                let v = hypot(acc.x, acc.y) / CGFloat(dtMs)
                let factor = 1.0 + opts.accelK * min(v / Self.vMax, 1.0)
                send(Protocol.moveFrame(dx: Int(acc.x * opts.sens * factor), dy: Int(acc.y * opts.sens * factor)))
                acc = .zero
            }
        case .scroll:
            if pinchMode {
                if pacc != 0 { send(Protocol.pinchFrame(dscale: Int(pacc), dx: 0, dy: 0)); pacc = 0 }
            } else if sacc != .zero {
                send(Protocol.scrollFrame(dx: Int(sacc.x * opts.scrollSpeed), dy: Int(sacc.y * opts.scrollSpeed)))
                sacc = .zero
            }
        default:
            break
        }
        switch state {
        case .pending, .move, .drag, .scroll, .three:
            break // 继续下一帧
        default:
            displayLink?.invalidate()
            displayLink = nil
        }
    }

    // ---- 惯性（指数衰减）----

    private func startInertia() {
        guard samples.count >= 2 else { return }
        let first = samples[0], last = samples[samples.count - 1]
        let dtMs = (last.t - first.t) * 1000
        guard dtMs >= 5 else { return }
        inertiaV = CGPoint(
            x: (last.x - first.x) / CGFloat(dtMs) * opts.scrollSpeed,
            y: (last.y - first.y) / CGFloat(dtMs) * opts.scrollSpeed
        )
        guard hypot(inertiaV.x, inertiaV.y) >= 0.05 else { return }
        inertiaLastT = CACurrentMediaTime()
        inertiaLink?.invalidate()
        let link = CADisplayLink(target: self, selector: #selector(inertiaStep))
        link.add(to: .main, forMode: .common)
        inertiaLink = link
    }

    @objc private func inertiaStep() {
        let now = CACurrentMediaTime()
        let dtMs = min(50, max(1, (now - inertiaLastT) * 1000))
        inertiaLastT = now
        let dx = inertiaV.x * CGFloat(dtMs), dy = inertiaV.y * CGFloat(dtMs)
        send(Protocol.scrollFrame(dx: Int(dx), dy: Int(dy)))
        let decay = pow(Self.inertiaLambda, dtMs / 16.7)
        inertiaV.x *= CGFloat(decay); inertiaV.y *= CGFloat(decay)
        if hypot(dx, dy) > Self.inertiaStopPx && hypot(inertiaV.x, inertiaV.y) > 0.02 {
            return // 继续
        }
        inertiaLink?.invalidate()
        inertiaLink = nil
    }
}
