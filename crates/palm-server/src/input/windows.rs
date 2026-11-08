//! Windows 输入注入后端：SendInput（§8.2）。
//!
//! 决策：
//! - **相对移动会被"提高指针精确度"二次加速** → 自管虚拟光标位置 +
//!   `MOUSEEVENTF_ABSOLUTE|VIRTUALDESK` 绝对坐标注入，跨多屏归一化 0–65535；
//! - 滚动：`MOUSEEVENTF_WHEEL`，`WHEEL_DELTA=120`/行，像素 → 行换算（正值=上滚）；
//! - 文本：`KEYEVENTF_UNICODE` 逐 UTF-16 码元注入（含代理对）；
//! - 高权限窗口（UAC/任务管理器）需以管理员运行——UIPI 会静默丢弃注入，
//!   文档提示用户（运行时无法可靠检测）。

use std::os::raw::c_int;

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT_0, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

use super::{Btn, InputBackend, Key, MediaKey};

mod vk {
    use super::VIRTUAL_KEY;
    pub const SHIFT: VIRTUAL_KEY = VIRTUAL_KEY(0x10);
    pub const CONTROL: VIRTUAL_KEY = VIRTUAL_KEY(0x11);
    pub const MENU: VIRTUAL_KEY = VIRTUAL_KEY(0x12); // Alt
    pub const LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5B);
    pub const CAPITAL: VIRTUAL_KEY = VIRTUAL_KEY(0x14);
    pub const ESCAPE: VIRTUAL_KEY = VIRTUAL_KEY(0x1B);
    pub const TAB: VIRTUAL_KEY = VIRTUAL_KEY(0x09);
    pub const SPACE: VIRTUAL_KEY = VIRTUAL_KEY(0x20);
    pub const RETURN: VIRTUAL_KEY = VIRTUAL_KEY(0x0D);
    pub const BACK: VIRTUAL_KEY = VIRTUAL_KEY(0x08);
    pub const DELETE: VIRTUAL_KEY = VIRTUAL_KEY(0x2E);
    pub const LEFT: VIRTUAL_KEY = VIRTUAL_KEY(0x25);
    pub const UP: VIRTUAL_KEY = VIRTUAL_KEY(0x26);
    pub const RIGHT: VIRTUAL_KEY = VIRTUAL_KEY(0x27);
    pub const DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0x28);
    pub const VOLUME_MUTE: VIRTUAL_KEY = VIRTUAL_KEY(0xAD);
    pub const VOLUME_DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0xAE);
    pub const VOLUME_UP: VIRTUAL_KEY = VIRTUAL_KEY(0xAF);
    pub const MEDIA_NEXT: VIRTUAL_KEY = VIRTUAL_KEY(0xB0);
    pub const MEDIA_PREV: VIRTUAL_KEY = VIRTUAL_KEY(0xB1);
    pub const MEDIA_PLAY_PAUSE: VIRTUAL_KEY = VIRTUAL_KEY(0xB3);
    /// 箭头/删除等扩展键需要 KEYEVENTF_EXTENDEDKEY
    pub fn is_extended(v: VIRTUAL_KEY) -> bool {
        matches!(
            v.0,
            0x21..=0x28 // PgUp/PgDn/End/Home/方向键
                | 0x2D | 0x2E // Ins/Del
                | 0x5B | 0x5C // LWin/RWin
        )
    }
}

/// 字符 → VK 码（美式布局）。None = 走 Unicode 注入路径。
fn vk_of_char(c: char) -> Option<VIRTUAL_KEY> {
    Some(match c {
        'a'..='z' | 'A'..='Z' => VIRTUAL_KEY(0x41 + (c.to_ascii_uppercase() as u16 - 'A' as u16)),
        '0'..='9' => VIRTUAL_KEY(0x30 + (c as u16 - '0' as u16)),
        '-' => VIRTUAL_KEY(0xBD),
        '=' => VIRTUAL_KEY(0xBB),
        '[' => VIRTUAL_KEY(0xDB),
        ']' => VIRTUAL_KEY(0xDD),
        ';' => VIRTUAL_KEY(0xBA),
        '\'' => VIRTUAL_KEY(0xDE),
        ',' => VIRTUAL_KEY(0xBC),
        '.' => VIRTUAL_KEY(0xBE),
        '/' => VIRTUAL_KEY(0xBF),
        '\\' => VIRTUAL_KEY(0xDC),
        '`' => VIRTUAL_KEY(0xC0),
        _ => return None,
    })
}

fn vk_of_key(k: Key) -> Option<VIRTUAL_KEY> {
    Some(match k {
        Key::Cmd => vk::LWIN,
        Key::Alt => vk::MENU,
        Key::Ctrl => vk::CONTROL,
        Key::Shift => vk::SHIFT,
        Key::Caps => vk::CAPITAL,
        Key::Enter => vk::RETURN,
        Key::Esc => vk::ESCAPE,
        Key::Tab => vk::TAB,
        Key::Space => vk::SPACE,
        Key::Backspace => vk::BACK,
        Key::Del => vk::DELETE,
        Key::Up => vk::UP,
        Key::Down => vk::DOWN,
        Key::Left => vk::LEFT,
        Key::Right => vk::RIGHT,
        Key::F(n) => VIRTUAL_KEY(0x70 + (n as u16 - 1)),
        Key::Char(c) => return vk_of_char(c),
        Key::Fn => return None, // Windows 无标准 Fn VK
    })
}

fn vk_of_media(m: MediaKey) -> VIRTUAL_KEY {
    match m {
        MediaKey::Play => vk::MEDIA_PLAY_PAUSE,
        MediaKey::Next => vk::MEDIA_NEXT,
        MediaKey::Prev => vk::MEDIA_PREV,
        MediaKey::VolUp => vk::VOLUME_UP,
        MediaKey::VolDown => vk::VOLUME_DOWN,
        MediaKey::Mute => vk::VOLUME_MUTE,
    }
}

pub struct WindowsBackend {
    /// 自管虚拟光标位置（屏幕坐标，含虚拟桌面偏移）
    x: i32,
    y: i32,
    /// 虚拟桌面包围盒（SM_XVIRTUALSCREEN 等）
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
}

impl WindowsBackend {
    pub fn new() -> Self {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            Self {
                x: pt.x,
                y: pt.y,
                vx,
                vy,
                vw,
                vh,
            }
        }
    }

    fn mouse(&self, flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, mouse_data: u32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: mouse_data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as c_int);
        }
    }

    /// 自管位置 → 绝对坐标（0–65535，跨虚拟桌面）
    fn send_abs_move(&self) {
        let nx = if self.vw > 1 {
            ((self.x - self.vx).clamp(0, self.vw - 1) as f64 * 65535.0 / (self.vw - 1) as f64) as i32
        } else {
            0
        };
        let ny = if self.vh > 1 {
            ((self.y - self.vy).clamp(0, self.vh - 1) as f64 * 65535.0 / (self.vh - 1) as f64) as i32
        } else {
            0
        };
        self.mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            nx,
            ny,
            0,
        );
    }

    fn keybd(&self, vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: if flags.0 & KEYEVENTF_UNICODE.0 != 0 {
                        VIRTUAL_KEY(0)
                    } else {
                        vk
                    },
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as c_int);
        }
    }
}

impl InputBackend for WindowsBackend {
    fn move_rel(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
        self.x = (self.x + dx).clamp(self.vx, self.vx + self.vw - 1);
        self.y = (self.y + dy).clamp(self.vy, self.vy + self.vh - 1);
        self.send_abs_move();
        Ok(())
    }

    fn move_abs(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        // 虚拟桌面归一化 0–65535 → 屏幕坐标
        self.x = self.vx + (self.vw * x / 65535).clamp(0, self.vw - 1);
        self.y = self.vy + (self.vh * y / 65535).clamp(0, self.vh - 1);
        self.send_abs_move();
        Ok(())
    }

    fn button(&mut self, btn: Btn, down: bool) -> anyhow::Result<()> {
        let flags = match (btn, down) {
            (Btn::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (Btn::Left, false) => MOUSEEVENTF_LEFTUP,
            (Btn::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (Btn::Right, false) => MOUSEEVENTF_RIGHTUP,
            (Btn::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (Btn::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        self.mouse(flags, 0, 0, 0);
        Ok(())
    }

    fn tap(&mut self, btn: Btn, count: u8) -> anyhow::Result<()> {
        // Windows 无 clickstate 字段：多击靠时序，两次点击间留小间隔让 App 识别双击
        for i in 0..count.max(1) {
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            self.button(btn, true)?;
            self.button(btn, false)?;
        }
        Ok(())
    }

    fn scroll_px(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
        // 语义：dy>0 = 视图向文档下方滚；dx>0 = 视图右移。
        // Windows wheelDelta：垂直正值 = 上滚 → 取反；水平正值 = 右移 → 同向。
        // wheelDelta 允许非 120 整数倍（1px = 1/120 行），支持平滑滚动，无需量化。
        let wy = (-dy).clamp(i16::MIN as i32, i16::MAX as i32);
        let wx = dx.clamp(i16::MIN as i32, i16::MAX as i32);
        if wx != 0 {
            self.mouse(MOUSEEVENTF_HWHEEL, 0, 0, wx as u32);
        }
        if wy != 0 {
            self.mouse(MOUSEEVENTF_WHEEL, 0, 0, wy as u32);
        }
        Ok(())
    }

    fn pinch(&mut self, dscale: i32, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        // Ctrl+滚轮缩放：先按住 Ctrl，发滚轮，再抬起
        self.key(Key::Ctrl, true)?;
        self.scroll_px(0, dscale)?;
        self.key(Key::Ctrl, false)?;
        Ok(())
    }

    fn key(&mut self, k: Key, down: bool) -> anyhow::Result<()> {
        let Some(v) = vk_of_key(k) else {
            anyhow::bail!("no VK code for {k:?}");
        };
        let mut flags = 0u32;
        if !down {
            flags |= KEYEVENTF_KEYUP.0;
        }
        if vk::is_extended(v) {
            flags |= KEYEVENTF_EXTENDEDKEY.0;
        }
        self.keybd(v, 0, KEYBD_EVENT_FLAGS(flags));
        Ok(())
    }

    fn text(&mut self, s: &str) -> anyhow::Result<()> {
        // KEYEVENTF_UNICODE 逐 UTF-16 码元；换行走 VK_RETURN
        for ch in s.encode_utf16() {
            if ch == 0x0A {
                self.keybd(vk::RETURN, 0, KEYBD_EVENT_FLAGS(0));
                self.keybd(vk::RETURN, 0, KEYEVENTF_KEYUP);
                continue;
            }
            self.keybd(VIRTUAL_KEY(0), ch, KEYEVENTF_UNICODE);
            self.keybd(VIRTUAL_KEY(0), ch, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
        }
        Ok(())
    }

    fn media(&mut self, m: MediaKey) -> anyhow::Result<()> {
        let v = vk_of_media(m);
        self.keybd(v, 0, KEYEVENTF_EXTENDEDKEY);
        self.keybd(v, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP);
        Ok(())
    }

    fn screen_size(&self) -> (i32, i32) {
        (self.vw, self.vh)
    }

    fn trusted(&self) -> bool {
        // Windows：SendInput 对高权限窗口静默失败（UIPI），无法可靠探测，默认 true
        true
    }
}
