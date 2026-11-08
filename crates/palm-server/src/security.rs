//! 安全层（§7）：一次性 token、可选 PIN、长期设备凭证、限流。
//!
//! 配对模型：
//! - 电脑端启动生成**一次性 token**（二维码承载），手机扫到后 `pair{mode:token}`；
//!   校验通过即消费（token 一次性），可选要求 PIN 二次确认；
//! - 配对成功签发**长期设备凭证 cred**（只存在手机 localStorage + 服务端设备表），
//!   重连走 `pair{mode:cred}` 免配对直进 active。

use rand::RngCore;

use crate::config::{Config, Device};

/// 每连接事件限流（§7：1000 事件/s，超限断开）
pub const RATE_LIMIT_EVENTS: u32 = 1000;
/// 单帧长度上限（字节）
pub const MAX_FRAME_LEN: usize = 1024;
/// 待配对超时（§4.4）
pub const PENDING_PAIR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// 心跳丢失超时
pub const HEARTBEAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 一次性 token 有效期
pub const ONE_TIME_TOKEN_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub fn random_hex(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    rand::rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// 待配对会话的中间状态（token 已验证、等待 PIN）。
#[derive(Debug, Default, Clone)]
pub struct PairProgress {
    pub token_ok: bool,
}

#[derive(Debug)]
pub struct Security {
    cfg_path: std::path::PathBuf,
    /// 一次性 token（hex16）+ 生成时间
    one_time: std::sync::Mutex<Option<(String, std::time::Instant)>>,
}

pub enum PairResult {
    /// 通过，签发新设备凭证
    Granted { device: Device },
    /// token 已通过，需要 PIN
    NeedPin,
    /// 拒绝
    Rejected(&'static str),
}

impl Security {
    pub fn new(cfg_path: std::path::PathBuf) -> Self {
        Self {
            cfg_path,
            one_time: std::sync::Mutex::new(None),
        }
    }

    /// 生成/轮换一次性 token，返回 token 字符串。
    pub fn rotate_one_time(&self) -> String {
        let token = random_hex(8);
        *self.one_time.lock().unwrap() = Some((token.clone(), std::time::Instant::now()));
        token
    }

    /// 当前一次性 token（不存在或过期返回 None）。
    pub fn peek_one_time(&self) -> Option<String> {
        let g = self.one_time.lock().unwrap();
        match &*g {
            Some((t, at)) if at.elapsed() < ONE_TIME_TOKEN_TTL => Some(t.clone()),
            _ => None,
        }
    }

    /// 校验并消费一次性 token。
    fn consume_one_time(&self, token: &str) -> bool {
        let mut g = self.one_time.lock().unwrap();
        match &*g {
            Some((t, at))
                if t == token && at.elapsed() < ONE_TIME_TOKEN_TTL =>
            {
                *g = None; // 一次性：立即作废
                true
            }
            _ => false,
        }
    }

    /// 处理配对请求（§9.1 时序）。
    ///
    /// `device_id` 是客户端自报的稳定设备标识（hello 承载）：同一设备重复配对时
    /// 更新既有记录并轮换 cred，保证已配对设备表里一台设备只有一条。
    #[allow(clippy::too_many_arguments)]
    pub fn pair(
        &self,
        cfg: &mut Config,
        mode: &str,
        token: Option<&str>,
        pin: Option<&str>,
        cred: Option<&str>,
        progress: &mut PairProgress,
        device_name_hint: &str,
        device_id: Option<&str>,
    ) -> PairResult {
        match mode {
            "token" => {
                let Some(token) = token else {
                    return PairResult::Rejected("missing_token");
                };
                if !self.consume_one_time(token) {
                    return PairResult::Rejected("bad_token");
                }
                progress.token_ok = true;
                if cfg.pairing.pin.is_empty() {
                    self.grant(cfg, device_name_hint, device_id)
                } else {
                    PairResult::NeedPin
                }
            }
            "pin" => {
                if !progress.token_ok {
                    return PairResult::Rejected("need_token");
                }
                match pin {
                    Some(p) if p == cfg.pairing.pin => {
                        self.grant(cfg, device_name_hint, device_id)
                    }
                    _ => PairResult::Rejected("bad_pin"),
                }
            }
            // 免配对码直连：知道 ip:port 即可配对（pairing.open 控制，PIN 仍生效）
            "open" => {
                if !cfg.pairing.open {
                    return PairResult::Rejected("open_disabled");
                }
                progress.token_ok = true;
                if cfg.pairing.pin.is_empty() {
                    self.grant(cfg, device_name_hint, device_id)
                } else {
                    PairResult::NeedPin
                }
            }
            "cred" => {
                let Some(cred) = cred else {
                    return PairResult::Rejected("missing_cred");
                };
                match cfg.device.iter().find(|d| d.cred == cred) {
                    Some(d) => PairResult::Granted {
                        device: d.clone(),
                    },
                    None => PairResult::Rejected("unknown_device"),
                }
            }
            _ => PairResult::Rejected("bad_mode"),
        }
    }

    fn grant(&self, cfg: &mut Config, name_hint: &str, device_id: Option<&str>) -> PairResult {
        // UA 看起来像 User-Agent 时用友好名，否则按原始字符串清理后使用
        let name = if name_hint.starts_with("Mozilla/") || name_hint.contains("Safari") || name_hint.contains("Chrome") {
            friendly_ua_name(name_hint)
        } else {
            sanitize_name(name_hint)
        };
        // 设备去重：同一 device_id 重复配对（重新扫码/清缓存/重装客户端）→
        // 更新既有记录（名称、cred 轮换、added 刷新），不追加新记录。
        let device = match device_id.and_then(|id| {
            cfg.device
                .iter_mut()
                .find(|d| d.device_id.as_deref() == Some(id))
        }) {
            Some(d) => {
                d.name = name;
                d.cred = random_hex(16); // 旧 cred 作废，新 cred 回发本连接
                d.added = chrono_now_iso();
                d.clone()
            }
            None => {
                let d = Device {
                    name,
                    cred: random_hex(16),
                    added: chrono_now_iso(),
                    device_id: device_id.map(str::to_owned),
                };
                cfg.device.push(d.clone());
                d
            }
        };
        if let Err(e) = cfg.save(&self.cfg_path) {
            tracing::warn!("persist device table failed: {e}");
        }
        PairResult::Granted { device }
    }

    /// 吊销设备（本地 API）。
    pub fn revoke(&self, cfg: &mut Config, cred: &str) -> bool {
        let before = cfg.device.len();
        cfg.device.retain(|d| d.cred != cred);
        let removed = cfg.device.len() != before;
        if removed {
            if let Err(e) = cfg.save(&self.cfg_path) {
                tracing::warn!("persist device table failed: {e}");
            }
        }
        removed
    }
}

fn sanitize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '/' | '，' | '、'))
        .take(40)
        .collect();
    if cleaned.is_empty() {
        "phone".into()
    } else {
        cleaned
    }
}

/// 从 User-Agent 提取友好设备名（"iPhone · Safari" / "Chrome · macOS"）。
pub fn friendly_ua_name(ua: &str) -> String {
    let device = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Macintosh") || ua.contains("Mac OS") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows"
    } else {
        ""
    };
    let browser = if ua.contains("MicroMessenger") {
        "WeChat"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        ""
    };
    match (device, browser) {
        ("", "") => "phone".into(),
        ("", b) => b.into(),
        (d, "") => d.into(),
        (d, b) => format!("{d} · {b}"),
    }
}

/// 无 chrono 依赖的本地 ISO 时间（够用即可，仅用于展示）。
fn chrono_now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单 UTC 格式化，不做完整历法换算（展示用途）
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 1970-01-01 起的天数 → y/m/d（简化版，容量足够到 2099）
    let mut y: i64 = 1970;
    let mut d = days as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mlens = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0;
    while d >= mlens[mo] {
        d -= mlens[mo];
        mo += 1;
    }
    format!("{y:04}-{:02}-{:02}T{h:02}:{m:02}:{s:02}Z", mo + 1, d + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cfg() -> (Security, std::path::PathBuf, Config) {
        let dir = std::env::temp_dir().join(format!("palm-test-{}", random_hex(4)));
        let path = dir.join("palm.toml");
        let cfg = Config::load_or_create(&path).unwrap();
        // 返回目录（非文件路径），方便测试收尾 remove_dir_all
        (Security::new(path), dir, cfg)
    }

    #[test]
    fn one_time_token_is_single_use() {
        let (sec, _dir, mut cfg) = tmp_cfg();
        let t = sec.rotate_one_time();
        assert!(sec.peek_one_time().is_some());
        let mut prog = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "token", Some(&t), None, None, &mut prog, "test-phone", None),
            PairResult::Granted { .. }
        ));
        // 二次使用同一 token 应被拒绝
        let mut prog2 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "token", Some(&t), None, None, &mut prog2, "x", None),
            PairResult::Rejected("bad_token")
        ));
        let _ = std::fs::remove_dir_all(_dir);
    }

    #[test]
    fn pin_flow() {
        let (sec, dir, mut cfg) = tmp_cfg();
        cfg.pairing.pin = "4821".into();
        let t = sec.rotate_one_time();
        let mut prog = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "token", Some(&t), None, None, &mut prog, "p", None),
            PairResult::NeedPin
        ));
        assert!(matches!(
            sec.pair(&mut cfg, "pin", None, Some("0000"), None, &mut prog, "p", None),
            PairResult::Rejected("bad_pin")
        ));
        assert!(matches!(
            sec.pair(&mut cfg, "pin", None, Some("4821"), None, &mut prog, "p", None),
            PairResult::Granted { .. }
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cred_flow_and_revoke() {
        let (sec, dir, mut cfg) = tmp_cfg();
        let t = sec.rotate_one_time();
        let mut prog = PairProgress::default();
        let dev = match sec.pair(&mut cfg, "token", Some(&t), None, None, &mut prog, "iphone", Some("dev-iphone")) {
            PairResult::Granted { device } => device,
            _ => panic!(),
        };
        let mut prog2 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "cred", None, None, Some(&dev.cred), &mut prog2, "x", None),
            PairResult::Granted { .. }
        ));
        assert!(sec.revoke(&mut cfg, &dev.cred));
        let mut prog3 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "cred", None, None, Some(&dev.cred), &mut prog3, "x", None),
            PairResult::Rejected("unknown_device")
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn re_pair_same_device_keeps_single_record() {
        let (sec, dir, mut cfg) = tmp_cfg();
        let t = sec.rotate_one_time();
        let mut prog = PairProgress::default();
        let d1 = match sec.pair(&mut cfg, "token", Some(&t), None, None, &mut prog, "phone-a", Some("dev-1")) {
            PairResult::Granted { device } => device,
            _ => panic!(),
        };
        // 同一设备再次配对（新 token）：记录数不变，cred 轮换
        let t2 = sec.rotate_one_time();
        let mut prog2 = PairProgress::default();
        let d2 = match sec.pair(&mut cfg, "token", Some(&t2), None, None, &mut prog2, "phone-a", Some("dev-1")) {
            PairResult::Granted { device } => device,
            _ => panic!(),
        };
        assert_eq!(cfg.device.len(), 1, "same device_id must not add records");
        assert_ne!(d1.cred, d2.cred, "cred should rotate on re-pair");
        // 旧 cred 失效、新 cred 可用
        let mut prog3 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "cred", None, None, Some(&d1.cred), &mut prog3, "x", None),
            PairResult::Rejected("unknown_device")
        ));
        let mut prog4 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "cred", None, None, Some(&d2.cred), &mut prog4, "x", None),
            PairResult::Granted { .. }
        ));
        // PIN 流程同样去重
        cfg.pairing.pin = "4821".into();
        let t3 = sec.rotate_one_time();
        let mut prog5 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "token", Some(&t3), None, None, &mut prog5, "phone-a", Some("dev-1")),
            PairResult::NeedPin
        ));
        // PIN 步骤复用同一 progress（token_ok 已在其中）
        assert!(matches!(
            sec.pair(&mut cfg, "pin", None, Some("4821"), None, &mut prog5, "phone-a", Some("dev-1")),
            PairResult::Granted { .. }
        ));
        assert_eq!(cfg.device.len(), 1);
        // 不同 device_id → 各自一条（先还原无 PIN 配置）
        cfg.pairing.pin.clear();
        let t4 = sec.rotate_one_time();
        let mut prog7 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "token", Some(&t4), None, None, &mut prog7, "phone-b", Some("dev-2")),
            PairResult::Granted { .. }
        ));
        assert_eq!(cfg.device.len(), 2);
        // 落盘后重读仍一致，且 device_id 持久化
        let reloaded = Config::load_or_create(&dir.join("palm.toml")).unwrap();
        assert_eq!(reloaded.device.len(), 2);
        assert_eq!(reloaded.device[0].device_id.as_deref(), Some("dev-1"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_pair_mode() {
        let (sec, dir, mut cfg) = tmp_cfg();
        // 默认开放直连 + 无 PIN → 知道地址即可配对，无需 token
        let mut prog = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "open", None, None, None, &mut prog, "phone-a", Some("dev-1")),
            PairResult::Granted { .. }
        ));
        assert_eq!(cfg.device.len(), 1);
        // 配了 PIN → open 也要二次确认，且同一设备仍去重
        cfg.pairing.pin = "1234".into();
        let mut prog2 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "open", None, None, None, &mut prog2, "phone-a", Some("dev-1")),
            PairResult::NeedPin
        ));
        assert!(matches!(
            sec.pair(&mut cfg, "pin", None, Some("1234"), None, &mut prog2, "phone-a", Some("dev-1")),
            PairResult::Granted { .. }
        ));
        assert_eq!(cfg.device.len(), 1, "open re-pair also dedupes");
        // 关闭开放直连 → 拒绝（只能走 token 扫码）
        cfg.pairing.open = false;
        let mut prog3 = PairProgress::default();
        assert!(matches!(
            sec.pair(&mut cfg, "open", None, None, None, &mut prog3, "phone-b", None),
            PairResult::Rejected("open_disabled")
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn friendly_names() {
        let ios = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
        assert_eq!(friendly_ua_name(ios), "iPhone · Safari");
        let mac_chrome = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";
        assert_eq!(friendly_ua_name(mac_chrome), "Mac · Chrome");
        let plain = "ws-test";
        assert_eq!(friendly_ua_name(plain), "phone");
    }
}
