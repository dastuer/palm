// Palm iOS 触控板底部的左右键区域（与 Android ClickZoneView 对应）：
// 按下=按下鼠标键，抬起=释放；配合触控板移动实现拖拽，轻点=单击。
// 多点触控：两根手指可分别按住左右键。

import UIKit

final class ClickZoneView: UIView {
    var onPress: ((_ btn: UInt8, _ down: Bool) -> Void)?

    private var activeTouches: [ObjectIdentifier: UInt8] = [:]
    private let dividerColor = UIColor(red: 0.137, green: 0.165, blue: 0.212, alpha: 1)   // #232A36
    private let highlightColor = UIColor(red: 0.36, green: 0.55, blue: 1.0, alpha: 0.16)  // 主题蓝 16%
    private let labelColor = UIColor(red: 0.243, green: 0.275, blue: 0.329, alpha: 1)     // #3E4654
    private let labelFont = UIFont.systemFont(ofSize: 20)

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = .clear
        isOpaque = false
    }

    required init?(coder: NSCoder) { fatalError("程序化构建，不支持 storyboard") }

    private func btn(at point: CGPoint) -> UInt8 {
        point.x < bounds.width / 2 ? Protocol.btnLeft : Protocol.btnRight
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for t in touches {
            let b = btn(at: t.location(in: self))
            activeTouches[ObjectIdentifier(t)] = b
            onPress?(b, true)
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }
        setNeedsDisplay()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        for t in touches {
            if let b = activeTouches.removeValue(forKey: ObjectIdentifier(t)) { onPress?(b, false) }
        }
        setNeedsDisplay()
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        touchesEnded(touches, with: event)
    }

    override func draw(_ rect: CGRect) {
        guard let c = UIGraphicsGetCurrentContext() else { return }
        let half = bounds.width / 2
        // 按下侧的高亮
        if activeTouches.values.contains(Protocol.btnLeft) {
            c.setFillColor(highlightColor.cgColor)
            c.fill(CGRect(x: 0, y: 1, width: half, height: bounds.height))
        }
        if activeTouches.values.contains(Protocol.btnRight) {
            c.setFillColor(highlightColor.cgColor)
            c.fill(CGRect(x: half, y: 1, width: half, height: bounds.height))
        }
        // 与触控板之间的水平分隔线 + 左右键之间的竖直分隔线
        c.setStrokeColor(dividerColor.cgColor)
        c.setLineWidth(1.5)
        c.move(to: CGPoint(x: 0, y: 0.75))
        c.addLine(to: CGPoint(x: bounds.width, y: 0.75))
        c.strokePath()
        c.move(to: CGPoint(x: half, y: 0.75))
        c.addLine(to: CGPoint(x: half, y: bounds.height))
        c.strokePath()
        // 文字标识（低调）
        let attrs: [NSAttributedString.Key: Any] = [.font: labelFont, .foregroundColor: labelColor]
        let midY = bounds.height / 2 - labelFont.lineHeight / 2
        ("左键" as NSString).draw(at: CGPoint(x: half / 2 - labelFont.size, y: midY), withAttributes: attrs)
        ("右键" as NSString).draw(at: CGPoint(x: half + half / 2 - labelFont.size, y: midY), withAttributes: attrs)
    }
}
