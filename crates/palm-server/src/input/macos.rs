//! macOS 输入注入后端：CoreGraphics CGEvent（§8.1）。
//!
//! 决策：
//! - 移动用绝对坐标注入（查询当前光标位置 + delta），避免二次加速干扰；
//! - 滚动走 `CGEventCreateScrollEvent2(kCGScrollEventUnitPixel)` 像素级事件，
//!   保留惯性帧，手感对齐原生触控板；
//! - 文本走 `CGEventKeyboardSetUnicodeString`，中文/emoji 直接可用；
//! - 媒体键走 NX keycode 特殊键码（ev_keymap.h）。
//!
//! 注入需要"辅助功能"权限：未授权时 CGEventPost 被系统静默忽略，
//! 启动时检测并给出深链引导（request_trust）。

use std::ffi::c_void;
use std::os::raw::{c_int, c_uchar, c_uint};

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

use super::{Btn, InputBackend, Key, MediaKey};

// --- CoreGraphics FFI -------------------------------------------------------

type CGEventRef = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGSize {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: c_uint,
        pos: CGPoint,
        button: c_uint,
    ) -> CGEventRef;
    fn CGEventPost(tap: c_uint, event: CGEventRef);
    fn CGEventSetIntegerValueField(event: CGEventRef, field: c_uint, value: i64);
    fn CGEventSetType(event: CGEventRef, t: c_uint);
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventCreateScrollWheelEvent(
        source: *const c_void,
        units: c_uint,
        wheel_count: c_uint,
        wheel1: i32,
        // wheel2/wheel3 为可变参数尾部（wheel_count>1 时传入）
        ...
    ) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, keydown: bool) -> CGEventRef;
    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, len: usize, string: *const u16);
    fn CGGetActiveDisplayList(max: c_uint, displays: *mut c_uint, count: *mut c_uint) -> c_int;
    fn CGDisplayBounds(display: c_uint) -> CGRect;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> c_uchar;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> c_uchar;
}

// --- 常量 -------------------------------------------------------------------

#[allow(dead_code)]
mod cg {
    pub const HID_EVENT_TAP: u32 = 0;
    pub const EVENT_MOUSE_MOVED: u32 = 5;
    pub const EVENT_LEFT_DOWN: u32 = 1;
    pub const EVENT_LEFT_UP: u32 = 2;
    pub const EVENT_LEFT_DRAGGED: u32 = 6;
    pub const EVENT_RIGHT_DOWN: u32 = 3;
    pub const EVENT_RIGHT_UP: u32 = 4;
    pub const EVENT_RIGHT_DRAGGED: u32 = 7;
    pub const EVENT_OTHER_DOWN: u32 = 25;
    pub const EVENT_OTHER_UP: u32 = 26;
    pub const EVENT_FLAGS_CHANGED: u32 = 14;
    pub const SCROLL_UNIT_PIXEL: u32 = 0;
    pub const FIELD_CLICK_STATE: u32 = 1;
    pub const MB_LEFT: u32 = 0;
    pub const MB_RIGHT: u32 = 1;
    pub const MB_CENTER: u32 = 2;
    pub const FLAG_SHIFT: u64 = 1 << 17;
    pub const FLAG_CTRL: u64 = 1 << 18;
    pub const FLAG_ALT: u64 = 1 << 19;
    pub const FLAG_CMD: u64 = 1 << 20;
    pub const FLAG_FN: u64 = 1 << 23;
}

/// kVK_* 键码（HIToolbox/Events.h）。
fn keycode_of(k: Key) -> Option<u16> {
    Some(match k {
        Key::Cmd => 0x37,
        Key::Shift => 0x38,
        Key::Alt => 0x3A,
        Key::Ctrl => 0x3B,
        Key::Fn => 0x3F,
        Key::Caps => 0x39,
        Key::Enter => 0x24,
        Key::Esc => 0x35,
        Key::Tab => 0x30,
        Key::Space => 0x31,
        Key::Backspace => 0x33,
        Key::Del => 0x75,
        Key::Up => 0x7E,
        Key::Down => 0x7D,
        Key::Left => 0x7B,
        Key::Right => 0x7C,
        Key::F(n) => match n {
            1 => 0x7A,
            2 => 0x78,
            3 => 0x63,
            4 => 0x76,
            5 => 0x60,
            6 => 0x61,
            7 => 0x62,
            8 => 0x64,
            9 => 0x65,
            10 => 0x6D,
            11 => 0x67,
            12 => 0x6F,
            _ => return None,
        },
        Key::Char(c) => match c {
            'a' => 0x00,
            's' => 0x01,
            'd' => 0x02,
            'f' => 0x03,
            'h' => 0x04,
            'g' => 0x05,
            'z' => 0x06,
            'x' => 0x07,
            'c' => 0x08,
            'v' => 0x09,
            'b' => 0x0B,
            'q' => 0x0C,
            'w' => 0x0D,
            'e' => 0x0E,
            'r' => 0x0F,
            'y' => 0x10,
            't' => 0x11,
            'u' => 0x20,
            'i' => 0x22,
            'o' => 0x1F,
            'p' => 0x23,
            'l' => 0x25,
            'j' => 0x26,
            'k' => 0x28,
            'n' => 0x2D,
            'm' => 0x2E,
            '1' => 0x12,
            '2' => 0x13,
            '3' => 0x14,
            '4' => 0x15,
            '5' => 0x17,
            '6' => 0x16,
            '7' => 0x1A,
            '8' => 0x1C,
            '9' => 0x19,
            '0' => 0x1D,
            '-' => 0x1B,
            '=' => 0x18,
            '[' => 0x21,
            ']' => 0x1E,
            ';' => 0x29,
            '\'' => 0x27,
            ',' => 0x2B,
            '.' => 0x2F,
            '/' => 0x2C,
            '\\' => 0x2A,
            '`' => 0x32,
            _ => return None,
        },
    })
}

fn modifier_flag(k: Key) -> Option<u64> {
    Some(match k {
        Key::Cmd => cg::FLAG_CMD,
        Key::Shift => cg::FLAG_SHIFT,
        Key::Alt => cg::FLAG_ALT,
        Key::Ctrl => cg::FLAG_CTRL,
        Key::Fn => cg::FLAG_FN,
        _ => return None,
    })
}

/// NX_KEYTYPE（IOKit hidsystem/ev_keymap.h），配合 kCGHIDEventTap 发布媒体键。
fn nx_media(m: MediaKey) -> u16 {
    match m {
        MediaKey::VolUp => 0,
        MediaKey::VolDown => 1,
        MediaKey::Mute => 7,
        MediaKey::Play => 16,
        MediaKey::Next => 17,
        MediaKey::Prev => 18,
    }
}

pub struct MacBackend {
    /// 当前按下的修饰键标志位（用于给后续键盘事件设置 flags）
    mod_flags: u64,
    /// 左键是否按住（按下期间移动发 Dragged 事件）
    left_down: bool,
    /// 主屏 bounds 缓存（启动时查询）
    bounds: CGRect,
}

impl MacBackend {
    pub fn new() -> Self {
        let bounds = unsafe {
            let mut ids = [0u32; 16];
            let mut count: u32 = 0;
            if CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) == 0 && count > 0 {
                let mut acc = CGDisplayBounds(ids[0]);
                for &id in &ids[1..count as usize] {
                    let b = CGDisplayBounds(id);
                    let min_x = acc.origin.x.min(b.origin.x);
                    let min_y = acc.origin.y.min(b.origin.y);
                    let max_x = (acc.origin.x + acc.size.w).max(b.origin.x + b.size.w);
                    let max_y = (acc.origin.y + acc.size.h).max(b.origin.y + b.size.h);
                    acc = CGRect {
                        origin: CGPoint { x: min_x, y: min_y },
                        size: CGSize {
                            w: max_x - min_x,
                            h: max_y - min_y,
                        },
                    };
                }
                acc
            } else {
                CGRect::default()
            }
        };
        Self {
            mod_flags: 0,
            left_down: false,
            bounds,
        }
    }

    fn cursor_pos(&self) -> CGPoint {
        unsafe {
            let ev = CGEventCreate(std::ptr::null());
            if ev.is_null() {
                return CGPoint::default();
            }
            let p = CGEventGetLocation(ev);
            CFRelease(ev);
            p
        }
    }

    fn mouse_event(&self, etype: u32, pos: CGPoint, button: u32, click_state: i64) {
        unsafe {
            let ev = CGEventCreateMouseEvent(
                std::ptr::null(),
                etype,
                pos,
                button,
            );
            if ev.is_null() {
                return;
            }
            if click_state > 1 {
                CGEventSetIntegerValueField(ev, cg::FIELD_CLICK_STATE, click_state);
            }
            CGEventPost(cg::HID_EVENT_TAP, ev);
            CFRelease(ev);
        }
    }
}

impl InputBackend for MacBackend {
    fn move_rel(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
        let cur = self.cursor_pos();
        let x = (cur.x + dx as f64)
            .clamp(self.bounds.origin.x, self.bounds.origin.x + self.bounds.size.w - 1.0);
        let y = (cur.y + dy as f64)
            .clamp(self.bounds.origin.y, self.bounds.origin.y + self.bounds.size.h - 1.0);
        let pos = CGPoint { x, y };
        if self.left_down {
            self.mouse_event(cg::EVENT_LEFT_DRAGGED, pos, cg::MB_LEFT, 1);
        } else {
            self.mouse_event(cg::EVENT_MOUSE_MOVED, pos, cg::MB_LEFT, 1);
        }
        Ok(())
    }

    fn move_abs(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        let pos = CGPoint {
            x: self.bounds.origin.x + self.bounds.size.w * (x as f64) / 65535.0,
            y: self.bounds.origin.y + self.bounds.size.h * (y as f64) / 65535.0,
        };
        if self.left_down {
            self.mouse_event(cg::EVENT_LEFT_DRAGGED, pos, cg::MB_LEFT, 1);
        } else {
            self.mouse_event(cg::EVENT_MOUSE_MOVED, pos, cg::MB_LEFT, 1);
        }
        Ok(())
    }

    fn button(&mut self, btn: Btn, down: bool) -> anyhow::Result<()> {
        let pos = self.cursor_pos();
        let (down_t, up_t, cgbtn) = match btn {
            Btn::Left => (cg::EVENT_LEFT_DOWN, cg::EVENT_LEFT_UP, cg::MB_LEFT),
            Btn::Right => (cg::EVENT_RIGHT_DOWN, cg::EVENT_RIGHT_UP, cg::MB_RIGHT),
            Btn::Middle => (cg::EVENT_OTHER_DOWN, cg::EVENT_OTHER_UP, cg::MB_CENTER),
        };
        self.mouse_event(if down { down_t } else { up_t }, pos, cgbtn, 1);
        if btn == Btn::Left {
            self.left_down = down;
        }
        Ok(())
    }

    fn tap(&mut self, btn: Btn, count: u8) -> anyhow::Result<()> {
        let pos = self.cursor_pos();
        let (down_t, up_t, cgbtn) = match btn {
            Btn::Left => (cg::EVENT_LEFT_DOWN, cg::EVENT_LEFT_UP, cg::MB_LEFT),
            Btn::Right => (cg::EVENT_RIGHT_DOWN, cg::EVENT_RIGHT_UP, cg::MB_RIGHT),
            Btn::Middle => (cg::EVENT_OTHER_DOWN, cg::EVENT_OTHER_UP, cg::MB_CENTER),
        };
        // 多击：完整 down/up × count，clickstate 递增（App 据此识别双击）
        for i in 1..=count.max(1) as i64 {
            self.mouse_event(down_t, pos, cgbtn, i);
            self.mouse_event(up_t, pos, cgbtn, i);
        }
        Ok(())
    }

    fn scroll_px(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
        // 语义约定：dy>0 = 视图向文档下方滚（已实测校准）；dx>0 = 视图右移
        // （与 Windows WM_MOUSEHWHEEL 同约定；水平方向未实测，如反向调此符号即可）。
        // CoreGraphics 像素滚动正值 = 向上滚，垂直取反注入。像素事件天然支持惯性帧（§8.1）。
        unsafe {
            let ev = CGEventCreateScrollWheelEvent(
                std::ptr::null(),
                cg::SCROLL_UNIT_PIXEL,
                2,
                -dy,
                dx,
            );
            if ev.is_null() {
                anyhow::bail!("CGEventCreateScrollWheelEvent failed");
            }
            CGEventPost(cg::HID_EVENT_TAP, ev);
            CFRelease(ev);
        }
        Ok(())
    }

    fn pinch(&mut self, dscale: i32, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        // 捏合 → Ctrl+像素滚动；dscale 正=张开=放大=向上滚（正值）
        unsafe {
            let ev = CGEventCreateScrollWheelEvent(
                std::ptr::null(),
                cg::SCROLL_UNIT_PIXEL,
                1,
                dscale,
            );
            if ev.is_null() {
                anyhow::bail!("CGEventCreateScrollWheelEvent failed");
            }
            CGEventSetFlags(ev, cg::FLAG_CTRL);
            CGEventPost(cg::HID_EVENT_TAP, ev);
            CFRelease(ev);
        }
        Ok(())
    }

    fn key(&mut self, k: Key, down: bool) -> anyhow::Result<()> {
        let code = keycode_of(k).ok_or_else(|| anyhow::anyhow!("no keycode for {k:?}"))?;
        let is_mod = modifier_flag(k);
        unsafe {
            let ev = CGEventCreateKeyboardEvent(std::ptr::null(), code, down);
            if ev.is_null() {
                anyhow::bail!("CGEventCreateKeyboardEvent failed");
            }
            if is_mod.is_some() {
                CGEventSetType(ev, cg::EVENT_FLAGS_CHANGED);
            }
            // 维护修饰键状态并同步 flags（组合键依赖）
            match (is_mod, down) {
                (Some(f), true) => self.mod_flags |= f,
                (Some(f), false) => self.mod_flags &= !f,
                _ => {}
            }
            CGEventSetFlags(ev, self.mod_flags);
            CGEventPost(cg::HID_EVENT_TAP, ev);
            CFRelease(ev);
        }
        Ok(())
    }

    fn text(&mut self, s: &str) -> anyhow::Result<()> {
        // 换行单独走回车键；其余按 UTF-16 分段注入（单事件上限约 20 单元）
        for part in s.split('\n') {
            if !part.is_empty() {
                self.text_chunk(part)?;
            }
            if s.contains('\n') {
                self.key(Key::Enter, true)?;
                self.key(Key::Enter, false)?;
            }
        }
        Ok(())
    }

    fn media(&mut self, m: MediaKey) -> anyhow::Result<()> {
        let code = nx_media(m);
        unsafe {
            for down in [true, false] {
                let ev = CGEventCreateKeyboardEvent(std::ptr::null(), code, down);
                if !ev.is_null() {
                    CGEventPost(cg::HID_EVENT_TAP, ev);
                    CFRelease(ev);
                }
            }
        }
        Ok(())
    }

    fn screen_size(&self) -> (i32, i32) {
        (
            self.bounds.size.w.round() as i32,
            self.bounds.size.h.round() as i32,
        )
    }

    fn trusted(&self) -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// 触发系统授权（§6.3.1）：AXIsProcessTrustedWithOptions(ask=true) 弹出
    /// "Palm 想要控制这台电脑" 系统对话框，点击"打开系统设置"后会把 Palm
    /// 自动登记进辅助功能列表（开关关闭态），用户拨开即授权。
    /// 返回 false 表示已无权限且未能弹出对话框（调用方可退回深链引导）。
    fn request_trust(&mut self) -> bool {
        unsafe {
            // kAXTrustedCheckOptionPrompt = "AXTrustedCheckOptionPrompt"
            let key = CFString::new("AXTrustedCheckOptionPrompt");
            let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
            let trusted = AXIsProcessTrustedWithOptions(
                options.as_concrete_TypeRef() as *const c_void,
            );
            if trusted != 0 {
                return true; // 已授权（竞态下正好变化）
            }
            // 弹窗已由系统展示（非阻塞）；再补一发深链，双保险直达面板
            let deep_link = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
            let _ = std::process::Command::new("open").arg(deep_link).spawn();
            false
        }
    }
}

impl MacBackend {
    fn text_chunk(&self, s: &str) -> anyhow::Result<()> {
        let units: Vec<u16> = s.encode_utf16().collect();
        for chunk in units.chunks(20) {
            unsafe {
                for down in [true, false] {
                    let ev = CGEventCreateKeyboardEvent(std::ptr::null(), 0, down);
                    if ev.is_null() {
                        anyhow::bail!("CGEventCreateKeyboardEvent failed");
                    }
                    CGEventKeyboardSetUnicodeString(ev, chunk.len(), chunk.as_ptr());
                    CGEventSetFlags(ev, self.mod_flags);
                    CGEventPost(cg::HID_EVENT_TAP, ev);
                    CFRelease(ev);
                }
            }
        }
        Ok(())
    }
}
