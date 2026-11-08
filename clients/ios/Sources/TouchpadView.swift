// Palm iOS 触控板手势区（§5.1）：multipleTouchEnabled + 全事件接管

import UIKit

final class TouchpadView: UIView {
    var engine: GestureEngine?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = UIColor(red: 0.059, green: 0.067, blue: 0.082, alpha: 1)
        let hint = UILabel()
        hint.text = "触控板\n单指点按 · 双指滚动 · 长按拖拽"
        hint.numberOfLines = 0
        hint.textColor = UIColor(red: 0.55, green: 0.58, blue: 0.64, alpha: 0.6)
        hint.textAlignment = .center
        hint.translatesAutoresizingMaskIntoConstraints = false
        addSubview(hint)
        NSLayoutConstraint.activate([
            hint.centerXAnchor.constraint(equalTo: centerXAnchor),
            hint.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("storyboard not used")
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        engine?.touchesBegan(touches)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        engine?.touchesMoved(touches)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        engine?.touchesEnded(touches, cancelled: false)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        engine?.touchesEnded(touches, cancelled: true)
    }
}
