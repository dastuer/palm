package io.palm.client

import android.content.Context
import android.content.SharedPreferences
import java.util.UUID

/** 本地设置（对应 web 端 localStorage + 服务端 cfg 同步） */
class SettingsStore(ctx: Context) {
    private val sp: SharedPreferences = ctx.getSharedPreferences("palm", Context.MODE_PRIVATE)

    /** 稳定设备标识：重复配对时服务端按它去重，一台设备只占一条已配对记录 */
    var deviceId: String
        get() = sp.getString("deviceId", null) ?: UUID.randomUUID().toString().also { deviceId = it }
        set(v) = sp.edit().putString("deviceId", v).apply()

    /** "ip:port"，null = 未配置 */
    var serverAddress: String?
        get() = sp.getString("server", null)
        set(v) = sp.edit().putString("server", v).apply()

    /** 一次性配对 token（URL #/t= 承载） */
    var token: String?
        get() = sp.getString("token", null)
        set(v) = sp.edit().putString("token", v).apply()

    /** 长期设备凭证（pair_ok 后由服务端签发，见 §7） */
    var cred: String?
        get() = sp.getString("cred", null)
        set(v) = sp.edit().putString("cred", v).apply()

    var sens: Float get() = sp.getFloat("sens", 1.0f); set(v) = sp.edit().putFloat("sens", v).apply()
    var scrollSpeed: Float get() = sp.getFloat("scrollSpeed", 1.0f); set(v) = sp.edit().putFloat("scrollSpeed", v).apply()
    var natural: Boolean get() = sp.getBoolean("natural", true); set(v) = sp.edit().putBoolean("natural", v).apply()
    var tapEnabled: Boolean get() = sp.getBoolean("tap", true); set(v) = sp.edit().putBoolean("tap", v).apply()
    var precise: Boolean get() = sp.getBoolean("precise", false); set(v) = sp.edit().putBoolean("precise", v).apply()
    var longPressMs: Int get() = sp.getInt("longPressMs", 450); set(v) = sp.edit().putInt("longPressMs", v).apply()

    var displayWidth: Int = 0
    var displayHeight: Int = 0
    var density: Float = 1f
}
