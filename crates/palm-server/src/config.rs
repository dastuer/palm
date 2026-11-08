//! 配置读写（palm.toml，TOML 人类可读，§3）。
//!
//! 手机端 `cfg` 消息会更新 sens/natural/scroll_speed/三指映射并持久化到本文件。
//! 文件权限 600（§7 安全设计）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCfg {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_name")]
    pub name: String,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            port: default_port(),
            name: default_name(),
        }
    }
}

fn default_port() -> u16 {
    8787
}
fn default_name() -> String {
    "Palm".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InputCfg {
    /// 指针灵敏度（客户端应用；cfg 消息同步）
    pub sens: f32,
    /// 自然滚动（mac 触控板方向）
    pub natural: bool,
    /// 滚动速度系数
    pub scroll_speed: f32,
}

impl Default for InputCfg {
    fn default() -> Self {
        Self {
            sens: 1.0,
            natural: true,
            scroll_speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PairingCfg {
    /// 4 位 PIN；空 = 关闭二次确认（§13 开放问题 2 的默认结论）
    pub pin: String,
    /// 免配对码直连：知道 ip:port 即可连接并自动配对（配了 PIN 仍需 PIN 二次确认）。
    /// 关闭后只能通过一次性 token（二维码）配对。
    pub open: bool,
}

impl Default for PairingCfg {
    fn default() -> Self {
        Self {
            pin: String::new(),
            open: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GestureCfg {
    pub g3_up: String,
    pub g3_down: String,
    pub g3_left: String,
    pub g3_right: String,
}

impl Default for GestureCfg {
    fn default() -> Self {
        Self {
            g3_up: "mission_control".into(),
            g3_down: "app_expose".into(),
            g3_left: "space_left".into(),
            g3_right: "space_right".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub cred: String,
    pub added: String,
    /// 客户端自报的稳定设备标识：同一物理设备无论配对多少次只保留一条记录
    /// （重复配对时更新本记录并轮换 cred，不新增）。旧配置文件无此字段 → None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerCfg,
    pub input: InputCfg,
    pub pairing: PairingCfg,
    pub gesture: GestureCfg,
    pub device: Vec<Device>,
}

impl Config {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            let cfg: Config = toml::from_str(&raw)?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

/// 默认配置路径：unix ~/.config/palm/palm.toml，windows %APPDATA%\palm\palm.toml。
pub fn default_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("palm").join("palm.toml");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("palm").join("palm.toml");
        }
    }
    PathBuf::from("palm.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cfg = Config::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.server.port, 8787);
        assert!(back.input.natural);
        assert_eq!(back.gesture.g3_up, "mission_control");
    }
}
