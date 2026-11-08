//! 输入注入后端抽象（§6.2 trait InputBackend）。
//!
//! 服务端把"语义事件"（位移/滚动/点击/文本）翻译为各平台的注入调用。
//! macOS 首发完整实现（input/macos.rs）；Windows/Linux 为 M4 里程碑。

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub mod stub;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Left,
    Right,
    Middle,
}

impl Btn {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Btn::Left,
            1 => Btn::Right,
            2 => Btn::Middle,
            3 | 4 => Btn::Middle, // x1/x2 暂映射 Middle（浏览器侧不使用）
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    Play,
    Next,
    Prev,
    VolUp,
    VolDown,
    Mute,
}

/// 可注入的按键（具名键 + 单字符键码映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Cmd,
    Alt,
    Ctrl,
    Shift,
    Fn,
    Caps,
    Enter,
    Esc,
    Tab,
    Space,
    Backspace,
    Del,
    Up,
    Down,
    Left,
    Right,
    F(u8),
    Char(char),
}

impl Key {
    /// 修饰键（按下后影响后续按键的 flags/state，各平台需维护状态）
    #[allow(dead_code)] // 仅 linux x11 路径使用
    pub fn is_modifier(&self) -> bool {
        matches!(self, Key::Cmd | Key::Alt | Key::Ctrl | Key::Shift | Key::Fn)
    }
}

/// 前端按键名 → Key（§4.2 key/tap/combo 中的 "k"）。
pub fn parse_key(s: &str) -> Option<Key> {
    Some(match s {
        "cmd" | "super" => Key::Cmd,
        "opt" | "alt" => Key::Alt,
        "ctrl" | "control" => Key::Ctrl,
        "shift" => Key::Shift,
        "fn" => Key::Fn,
        "caps" | "capslock" => Key::Caps,
        "enter" | "return" => Key::Enter,
        "esc" => Key::Esc,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" | "bs" => Key::Backspace,
        "delete" | "del" => Key::Del,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        s if s.len() == 2 && s.starts_with('f') => {
            let n: u8 = s[1..].parse().ok()?;
            if (1..=12).contains(&n) {
                Key::F(n)
            } else {
                return None;
            }
        }
        s => {
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() || !c.is_ascii_graphic() {
                return None;
            }
            Key::Char(c)
        }
    })
}

pub fn parse_media(s: &str) -> Option<MediaKey> {
    Some(match s {
        "play" | "pause" | "playpause" => MediaKey::Play,
        "next" => MediaKey::Next,
        "prev" => MediaKey::Prev,
        "volup" => MediaKey::VolUp,
        "voldown" => MediaKey::VolDown,
        "mute" => MediaKey::Mute,
        _ => return None,
    })
}

pub trait InputBackend: Send {
    /// 相对移动（dx/dy 已含灵敏度，服务端换算为绝对坐标注入）
    fn move_rel(&mut self, dx: i32, dy: i32) -> anyhow::Result<()>;
    /// 绝对移动（虚拟桌面归一化 ×65535）
    fn move_abs(&mut self, x: i32, y: i32) -> anyhow::Result<()>;
    fn button(&mut self, btn: Btn, down: bool) -> anyhow::Result<()>;
    /// 合成点击（count: 1=单击 2=双击）
    fn tap(&mut self, btn: Btn, count: u8) -> anyhow::Result<()>;
    /// 像素级滚动（服务端按平台换算单位）
    fn scroll_px(&mut self, dx: i32, dy: i32) -> anyhow::Result<()>;
    /// 双指捏合 → Ctrl+滚轮（dscale 正=张开=放大）
    fn pinch(&mut self, dscale: i32, dx: i32, dy: i32) -> anyhow::Result<()>;
    fn key(&mut self, k: Key, down: bool) -> anyhow::Result<()>;
    /// Unicode 整句注入（中文/emoji）
    fn text(&mut self, s: &str) -> anyhow::Result<()>;
    fn media(&mut self, m: MediaKey) -> anyhow::Result<()>;
    /// 虚拟桌面尺寸（px，ABS 模式使用，backlog）
    #[allow(dead_code)]
    fn screen_size(&self) -> (i32, i32);
    /// 平台注入权限是否就绪（macOS 辅助功能等）
    fn trusted(&self) -> bool;
    /// 权限引导（打开系统设置面板等）；返回 false 表示无需引导
    fn request_trust(&mut self) -> bool {
        false
    }
}

/// 按编译目标构造后端。
pub fn create_backend() -> Box<dyn InputBackend> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacBackend::new());
    #[cfg(target_os = "windows")]
    return Box::new(windows::WindowsBackend::new());
    #[cfg(target_os = "linux")]
    return linux::create_backend();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Box::new(stub::StubBackend);
}
