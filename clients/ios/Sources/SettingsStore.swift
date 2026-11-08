// Palm iOS 本地设置（对应 web localStorage + 服务端 cfg 同步）

import Foundation

final class SettingsStore {
    static let shared = SettingsStore()
    private let d = UserDefaults.standard

    /// "ip:port"，nil = 未配置
    var serverAddress: String? {
        get { d.string(forKey: "server") }
        set { d.set(newValue, forKey: "server") }
    }

    /// 一次性配对 token
    var token: String? {
        get { d.string(forKey: "token") }
        set { d.set(newValue, forKey: "token") }
    }

    /// 长期设备凭证（pair_ok 后由服务端签发）
    var cred: String? {
        get { d.string(forKey: "cred") }
        set { d.set(newValue, forKey: "cred") }
    }

    /// 稳定设备标识：重复配对时服务端按它去重，一台设备只占一条已配对记录
    var deviceId: String {
        get {
            if let s = d.string(forKey: "deviceId") { return s }
            let s = UUID().uuidString
            d.set(s, forKey: "deviceId")
            return s
        }
    }

    var sens: Double {
        get { d.double(forKey: "sens") == 0 ? 1.0 : d.double(forKey: "sens") }
        set { d.set(newValue, forKey: "sens") }
    }
    var scrollSpeed: Double {
        get { d.double(forKey: "scrollSpeed") == 0 ? 1.0 : d.double(forKey: "scrollSpeed") }
        set { d.set(newValue, forKey: "scrollSpeed") }
    }
    var natural: Bool {
        get { d.object(forKey: "natural") as? Bool ?? true }
        set { d.set(newValue, forKey: "natural") }
    }
    var tapEnabled: Bool {
        get { d.object(forKey: "tap") as? Bool ?? true }
        set { d.set(newValue, forKey: "tap") }
    }
    var precise: Bool {
        get { d.bool(forKey: "precise") }
        set { d.set(newValue, forKey: "precise") }
    }
    var longPressMs: Double {
        get { d.double(forKey: "longPressMs") == 0 ? 450 : d.double(forKey: "longPressMs") }
        set { d.set(newValue, forKey: "longPressMs") }
    }
}
