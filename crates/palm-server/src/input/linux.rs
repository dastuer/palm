//! Linux 输入注入后端（§8.3）：
//! - **X11**：x11rb（纯 Rust）+ XTEST 扩展，主路径；
//! - **Wayland / 无 X**：/dev/uinput 内核级虚拟设备（需 input 组或 udev 规则），实验性；
//! - 运行时选择：DISPLAY 存在优先 X11，连接失败或无 DISPLAY 用 uinput，再不行落到 stub。
//!
//! 已知限制（§13）：Unicode 文本注入依赖键盘布局含对应 keysym，非拉丁文在 X11/uinput
//! 路径都不可靠——返回错误并记录日志，不做静默失败。

use crate::input::InputBackend;

/// 核心事件类型号（X11 protocol：x11rb 不导出，见 X Protocol Event Types）
const X_KEY_PRESS: u8 = 2;
const X_KEY_RELEASE: u8 = 3;
const X_BUTTON_PRESS: u8 = 4;
const X_BUTTON_RELEASE: u8 = 5;
const X_MOTION_NOTIFY: u8 = 6;

/// 滚动：X11 按钮事件/滚轮刻度与像素的换算（可按手感调整）
const SCROLL_PX_PER_STEP: f32 = 40.0;

// ---------------------------------------------------------------------------
// X11（XTEST）
// ---------------------------------------------------------------------------

mod x11 {
    use std::collections::HashMap;

    use super::{X_BUTTON_PRESS, X_BUTTON_RELEASE, X_KEY_PRESS, X_KEY_RELEASE, X_MOTION_NOTIFY};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::protocol::xtest::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    use crate::input::{Btn, InputBackend, Key, MediaKey};
    use super::SCROLL_PX_PER_STEP;

    // keysym 常量（keysymdef.h）
    mod ks {
        pub const SPACE: u32 = 0x20;
        pub const TAB: u32 = 0xff09;
        pub const RETURN: u32 = 0xff0d;
        pub const ESCAPE: u32 = 0xff1b;
        pub const BACKSPACE: u32 = 0xff08;
        pub const DELETE: u32 = 0xffff;
        pub const LEFT: u32 = 0xff51;
        pub const UP: u32 = 0xff52;
        pub const RIGHT: u32 = 0xff53;
        pub const DOWN: u32 = 0xff54;
        pub const SHIFT_L: u32 = 0xffe1;
        pub const CONTROL_L: u32 = 0xffe3;
        pub const CAPS_LOCK: u32 = 0xffe5;
        pub const ALT_L: u32 = 0xffe9;
        pub const SUPER_L: u32 = 0xffeb;
        pub const F1: u32 = 0xffbe;
    }

    fn keysym_of_key(k: Key) -> Option<u32> {
        Some(match k {
            Key::Cmd => ks::SUPER_L,
            Key::Alt => ks::ALT_L,
            Key::Ctrl => ks::CONTROL_L,
            Key::Shift => ks::SHIFT_L,
            Key::Caps => ks::CAPS_LOCK,
            Key::Enter => ks::RETURN,
            Key::Esc => ks::ESCAPE,
            Key::Tab => ks::TAB,
            Key::Space => ks::SPACE,
            Key::Backspace => ks::BACKSPACE,
            Key::Del => ks::DELETE,
            Key::Up => ks::UP,
            Key::Down => ks::DOWN,
            Key::Left => ks::LEFT,
            Key::Right => ks::RIGHT,
            Key::F(n) => ks::F1 + (n as u32 - 1),
            Key::Char(c) if c.is_ascii() => c as u32,
            Key::Fn | Key::Char(_) => return None,
        })
    }

    pub struct X11Backend {
        conn: RustConnection,
        /// keysym → (keycode, 是否 shifted 列)
        keymap: HashMap<u32, (u8, bool)>,
        /// 服务端当前按住的修饰键（X server 侧 modifier 状态由 XTEST 按键事件维护）
        held_mods: Vec<Key>,
        scroll_acc_x: f32,
        scroll_acc_y: f32,
        root: u32,
        w: u16,
        h: u16,
    }

    impl X11Backend {
        pub fn connect() -> anyhow::Result<Self> {
            let (conn, screen) = x11rb::connect(None)?;
            // XTEST 扩展可用性检查
            let xtest = conn.xtest_get_version(2, 2)?.reply()?;
            if xtest.major_version < 2 {
                anyhow::bail!("XTEST extension not available");
            }
            let setup = conn.setup();
            let root = setup.roots[screen].root;
            let w = setup.roots[screen].width_in_pixels;
            let h = setup.roots[screen].height_in_pixels;
            // 构建 keysym → keycode 映射
            let (min, max) = (setup.min_keycode, setup.max_keycode);
            let map = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
            let per = map.keysyms_per_keycode as usize;
            let mut keymap = HashMap::new();
            for (i, chunk) in map.keysyms.chunks(per).enumerate() {
                let keycode = min + i as u8;
                for (col, ks) in chunk.iter().enumerate() {
                    if *ks != 0 {
                        keymap.entry(*ks).or_insert((keycode, col >= 1));
                    }
                }
            }
            Ok(Self {
                conn,
                keymap,
                held_mods: Vec::new(),
                scroll_acc_x: 0.0,
                scroll_acc_y: 0.0,
                root,
                w,
                h,
            })
        }

        fn fake_key(&self, keycode: u8, press: bool) -> anyhow::Result<()> {
            self.conn
                .xtest_fake_input(
                    if press { X_KEY_PRESS } else { X_KEY_RELEASE },
                    keycode,
                    0,
                    self.root,
                    0,
                    0,
                    0,
                )?
                .check()?;
            Ok(())
        }

        /// 按下 keysym（必要时自动补 shift）
        fn press_keysym(&mut self, keysym: u32, press: bool) -> anyhow::Result<()> {
            let Some(&(code, shifted)) = self.keymap.get(&keysym) else {
                anyhow::bail!("keysym 0x{keysym:x} not in keyboard mapping");
            };
            let shift_held = self.held_mods.contains(&Key::Shift);
            if press {
                if shifted && !shift_held {
                    if let Some(&(sc, _)) = self.keymap.get(&ks::SHIFT_L) {
                        self.fake_key(sc, true)?;
                    }
                }
                self.fake_key(code, true)?;
            } else {
                self.fake_key(code, false)?;
                if shifted && !shift_held {
                    if let Some(&(sc, _)) = self.keymap.get(&ks::SHIFT_L) {
                        self.fake_key(sc, false)?;
                    }
                }
            }
            Ok(())
        }

        fn button_click(&self, btn: u8, press: bool) -> anyhow::Result<()> {
            let t = if press { X_BUTTON_PRESS } else { X_BUTTON_RELEASE };
            self.conn.xtest_fake_input(t, btn, 0, self.root, 0, 0, 0)?.check()?;
            Ok(())
        }
    }

    impl InputBackend for X11Backend {
        fn move_rel(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
            // detail=1 → 相对移动（x/y 为增量），无 round-trip
            self.conn
                .xtest_fake_input(X_MOTION_NOTIFY, 1, 0, self.root, dx as i16, dy as i16, 0)?
                .check()?;
            Ok(())
        }

        fn move_abs(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
            let ax = (x as f64 / 65535.0 * self.w as f64) as i16;
            let ay = (y as f64 / 65535.0 * self.h as f64) as i16;
            self.conn
                .xtest_fake_input(X_MOTION_NOTIFY, 0, 0, self.root, ax, ay, 0)?
                .check()?;
            Ok(())
        }

        fn button(&mut self, btn: Btn, down: bool) -> anyhow::Result<()> {
            let b = match btn {
                Btn::Left => 1,
                Btn::Middle => 2,
                Btn::Right => 3,
            };
            self.button_click(b, down)
        }

        fn tap(&mut self, btn: Btn, count: u8) -> anyhow::Result<()> {
            let b = match btn {
                Btn::Left => 1,
                Btn::Middle => 2,
                Btn::Right => 3,
            };
            for i in 0..count.max(1) {
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                self.button_click(b, true)?;
                self.button_click(b, false)?;
            }
            Ok(())
        }

        fn scroll_px(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
            // 语义：dy>0 = 视图下移 → 滚轮按钮 5（下）；dx>0 = 视图右移 → 按钮 7（右）
            self.scroll_acc_y += dy as f32;
            self.scroll_acc_x += dx as f32;
            let steps_y = (self.scroll_acc_y / SCROLL_PX_PER_STEP).trunc();
            let steps_x = (self.scroll_acc_x / SCROLL_PX_PER_STEP).trunc();
            self.scroll_acc_y -= steps_y * SCROLL_PX_PER_STEP;
            self.scroll_acc_x -= steps_x * SCROLL_PX_PER_STEP;
            for _ in 0..steps_y.abs() as i32 {
                // 值为正 = 向下滚（按钮 5），为负 = 向上（按钮 4）
                self.button_click(if steps_y > 0.0 { 5 } else { 4 }, true)?;
                self.button_click(if steps_y > 0.0 { 5 } else { 4 }, false)?;
            }
            for _ in 0..steps_x.abs() as i32 {
                self.button_click(if steps_x > 0.0 { 7 } else { 6 }, true)?;
                self.button_click(if steps_x > 0.0 { 7 } else { 6 }, false)?;
            }
            Ok(())
        }

        fn pinch(&mut self, dscale: i32, _dx: i32, _dy: i32) -> anyhow::Result<()> {
            self.key(Key::Ctrl, true)?;
            self.scroll_px(0, dscale)?;
            self.key(Key::Ctrl, false)?;
            Ok(())
        }

        fn key(&mut self, k: Key, down: bool) -> anyhow::Result<()> {
            let Some(keysym) = keysym_of_key(k) else {
                anyhow::bail!("no keysym for {k:?}");
            };
            if k.is_modifier() {
                if down {
                    self.held_mods.push(k);
                } else {
                    self.held_mods.retain(|m| *m != k);
                }
                // 修饰键本身一定在未移位列
                let Some(&(code, _)) = self.keymap.get(&keysym) else {
                    anyhow::bail!("modifier keysym 0x{keysym:x} missing");
                };
                self.fake_key(code, down)?;
                return Ok(());
            }
            self.press_keysym(keysym, down)
        }

        fn text(&mut self, s: &str) -> anyhow::Result<()> {
            for ch in s.chars() {
                if ch == '\n' {
                    self.key(Key::Enter, true)?;
                    self.key(Key::Enter, false)?;
                    continue;
                }
                // Latin-1 直接映射；BMP 用 keysym unicode 段（0x01000000 + u）
                let keysym = if (ch as u32) <= 0xff {
                    ch as u32
                } else {
                    0x0100_0000 + ch as u32
                };
                self.press_keysym(keysym, true)?;
                self.press_keysym(keysym, false)?;
            }
            Ok(())
        }

        fn media(&mut self, m: MediaKey) -> anyhow::Result<()> {
            // XF86 多媒体 keysym
            let keysym = match m {
                MediaKey::Play => 0x1008_ff14,     // XF86AudioPlay
                MediaKey::Next => 0x1008_ff17,     // XF86AudioNext
                MediaKey::Prev => 0x1008_ff16,     // XF86AudioPrev
                MediaKey::VolUp => 0x1008_ff13,    // XF86AudioRaiseVolume
                MediaKey::VolDown => 0x1008_ff11,  // XF86AudioLowerVolume
                MediaKey::Mute => 0x1008_ff12,     // XF86AudioMute
            };
            // 多媒体键通常只在 col0 缺失时存在；直接查映射，查不到即报错
            let Some(&(code, shifted)) = self.keymap.get(&keysym) else {
                anyhow::bail!("media keysym 0x{keysym:x} not mapped");
            };
            let _ = shifted;
            self.fake_key(code, true)?;
            self.fake_key(code, false)?;
            Ok(())
        }

        fn screen_size(&self) -> (i32, i32) {
            (self.w as i32, self.h as i32)
        }

        fn trusted(&self) -> bool {
            // 能连上并校验 XTEST 即视为可用
            true
        }
    }
}

// ---------------------------------------------------------------------------
// uinput（Wayland / 无 X）
// ---------------------------------------------------------------------------

mod uinput {
    use std::fs::File;
    use std::io::Write;
    use std::os::fd::AsRawFd;

    use crate::input::{Btn, InputBackend, Key, MediaKey};
    use super::SCROLL_PX_PER_STEP;

    // linux/uinput.h ioctl
    const UI_SET_EVBIT: u64 = 0x4004_5564;
    const UI_SET_KEYBIT: u64 = 0x4004_5565;
    const UI_SET_RELBIT: u64 = 0x4004_5566;
    const UI_DEV_CREATE: u64 = 0x5501;

    // linux/input-event-codes.h
    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_REL: u16 = 0x02;
    const REL_X: u16 = 0x00;
    const REL_Y: u16 = 0x01;
    const REL_HWHEEL: u16 = 0x06;
    const REL_WHEEL: u16 = 0x08;
    const REL_WHEEL_HI_RES: u16 = 0x0b;
    const REL_HWHEEL_HI_RES: u16 = 0x0c;
    const SYN_REPORT: u16 = 0;
    const BTN_LEFT: u16 = 0x110;
    const BTN_RIGHT: u16 = 0x111;
    const BTN_MIDDLE: u16 = 0x112;

    mod kc {
        pub const ESC: u16 = 1;
        pub const TAB: u16 = 15;
        pub const ENTER: u16 = 28;
        pub const SPACE: u16 = 57;
        pub const BACKSPACE: u16 = 14;
        pub const DELETE: u16 = 111;
        pub const LEFT: u16 = 105;
        pub const RIGHT: u16 = 106;
        pub const UP: u16 = 103;
        pub const DOWN: u16 = 108;
        pub const LEFTCTRL: u16 = 29;
        pub const LEFTSHIFT: u16 = 42;
        pub const LEFTALT: u16 = 56;
        pub const LEFTMETA: u16 = 125;
        pub const CAPSLOCK: u16 = 58;
        pub const COMMA: u16 = 51;
        pub const DOT: u16 = 52;
        pub const SLASH: u16 = 53;
        pub const SEMICOLON: u16 = 39;
        pub const APOSTROPHE: u16 = 40;
        pub const MINUS: u16 = 12;
        pub const EQUAL: u16 = 13;
        pub const LEFTBRACE: u16 = 26;
        pub const RIGHTBRACE: u16 = 27;
        pub const BACKSLASH: u16 = 43;
        pub const GRAVE: u16 = 41;
        pub const PLAYPAUSE: u16 = 164;
        pub const NEXTSONG: u16 = 171;
        pub const PREVIOUSSONG: u16 = 165;
        pub const VOLUMEUP: u16 = 115;
        pub const VOLUMEDOWN: u16 = 114;
        pub const MUTE: u16 = 113;
        /// 字母/数字/F 键的连续段起点
        pub const KEY_A: u16 = 30;
        pub const KEY_1: u16 = 2;
        pub const F1: u16 = 59;
    }

    fn keycode_of_key(k: Key) -> Option<u16> {
        Some(match k {
            Key::Cmd => kc::LEFTMETA,
            Key::Alt => kc::LEFTALT,
            Key::Ctrl => kc::LEFTCTRL,
            Key::Shift => kc::LEFTSHIFT,
            Key::Caps => kc::CAPSLOCK,
            Key::Enter => kc::ENTER,
            Key::Esc => kc::ESC,
            Key::Tab => kc::TAB,
            Key::Space => kc::SPACE,
            Key::Backspace => kc::BACKSPACE,
            Key::Del => kc::DELETE,
            Key::Up => kc::UP,
            Key::Down => kc::DOWN,
            Key::Left => kc::LEFT,
            Key::Right => kc::RIGHT,
            Key::F(n) => kc::F1 + u16::from(n - 1),
            Key::Char(c) => match c {
                // QWERTY 行非连续，逐行映射（linux/input-event-codes.h）
                'a' => 30, 's' => 31, 'd' => 32, 'f' => 33, 'g' => 34,
                'h' => 35, 'j' => 36, 'k' => 37, 'l' => 38,
                'q' => 16, 'w' => 17, 'e' => 18, 'r' => 19, 't' => 20,
                'y' => 21, 'u' => 22, 'i' => 23, 'o' => 24, 'p' => 25,
                'z' => 44, 'x' => 45, 'c' => 46, 'v' => 47, 'b' => 48,
                'n' => 49, 'm' => 50,
                '1'..='9' => kc::KEY_1 + (c as u16 - '1' as u16),
                '0' => 11,
                '-' => kc::MINUS,
                '=' => kc::EQUAL,
                '[' => kc::LEFTBRACE,
                ']' => kc::RIGHTBRACE,
                ';' => kc::SEMICOLON,
                '\'' => kc::APOSTROPHE,
                ',' => kc::COMMA,
                '.' => kc::DOT,
                '/' => kc::SLASH,
                '\\' => kc::BACKSLASH,
                '`' => kc::GRAVE,
                _ => return None,
            },
            Key::Fn => return None,
        })
    }

    fn keycode_of_media(m: MediaKey) -> u16 {
        match m {
            MediaKey::Play => kc::PLAYPAUSE,
            MediaKey::Next => kc::NEXTSONG,
            MediaKey::Prev => kc::PREVIOUSSONG,
            MediaKey::VolUp => kc::VOLUMEUP,
            MediaKey::VolDown => kc::VOLUMEDOWN,
            MediaKey::Mute => kc::MUTE,
        }
    }

    pub struct UinputBackend {
        dev: File,
        scroll_acc_x: f32,
        scroll_acc_y: f32,
    }

    impl UinputBackend {
        pub fn open() -> anyhow::Result<Self> {
            let mut dev = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/uinput")?;
            unsafe { uinput_setup(&mut dev)? };
            Ok(Self {
                dev,
                scroll_acc_x: 0.0,
                scroll_acc_y: 0.0,
            })
        }

        fn emit(&mut self, etype: u16, code: u16, value: i32) -> anyhow::Result<()> {
            // struct input_event { timeval(16B) + u16 type + u16 code + i32 value }，64 位 24B
            let mut buf = [0u8; 24];
            buf[16..18].copy_from_slice(&etype.to_le_bytes());
            buf[18..20].copy_from_slice(&code.to_le_bytes());
            buf[20..24].copy_from_slice(&value.to_le_bytes());
            self.dev.write_all(&buf)?;
            Ok(())
        }

        fn syn(&mut self) -> anyhow::Result<()> {
            self.emit(EV_SYN, SYN_REPORT, 0)
        }
    }

    unsafe fn uinput_setup(dev: &mut File) -> anyhow::Result<()> {
        let fd = dev.as_raw_fd();
        let ioctl = |req: u64, arg: u32| -> anyhow::Result<()> {
            let rc = unsafe { libc::ioctl(fd, req as libc::c_ulong, arg) };
            if rc < 0 {
                anyhow::bail!("uinput ioctl 0x{req:x} failed (errno {})", std::io::Error::last_os_error());
            }
            Ok(())
        };
        ioctl(UI_SET_EVBIT, EV_KEY as u32)?;
        ioctl(UI_SET_EVBIT, EV_REL as u32)?;
        ioctl(UI_SET_EVBIT, EV_SYN as u32)?;
        ioctl(UI_SET_RELBIT, REL_X as u32)?;
        ioctl(UI_SET_RELBIT, REL_Y as u32)?;
        ioctl(UI_SET_RELBIT, REL_WHEEL as u32)?;
        ioctl(UI_SET_RELBIT, REL_HWHEEL as u32)?;
        ioctl(UI_SET_RELBIT, REL_WHEEL_HI_RES as u32)?;
        ioctl(UI_SET_RELBIT, REL_HWHEEL_HI_RES as u32)?;
        // 注册会用到的所有按键位
        let mut keys = vec![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE];
        for c in [
            kc::ESC, kc::TAB, kc::ENTER, kc::SPACE, kc::BACKSPACE, kc::DELETE,
            kc::LEFT, kc::RIGHT, kc::UP, kc::DOWN,
            kc::LEFTCTRL, kc::LEFTSHIFT, kc::LEFTALT, kc::LEFTMETA, kc::CAPSLOCK,
            kc::COMMA, kc::DOT, kc::SLASH, kc::SEMICOLON, kc::APOSTROPHE,
            kc::MINUS, kc::EQUAL, kc::LEFTBRACE, kc::RIGHTBRACE, kc::BACKSLASH, kc::GRAVE,
            kc::PLAYPAUSE, kc::NEXTSONG, kc::PREVIOUSSONG, kc::VOLUMEUP, kc::VOLUMEDOWN, kc::MUTE,
        ] {
            keys.push(c);
        }
        for i in 0..26 {
            keys.push(kc::KEY_A + i);
        }
        for i in 0..10 {
            keys.push(kc::KEY_1 + i);
        }
        for i in 0..12 {
            keys.push(kc::F1 + i);
        }
        for k in keys {
            ioctl(UI_SET_KEYBIT, k as u32)?;
        }
        // struct uinput_user_dev：name[80] + input_id(16B) + ff(4B) + 4×64×4B abs 数组
        let mut setup = vec![0u8; 80 + 16 + 4 + 4 * 64 * 4];
        setup[..4].copy_from_slice(b"palm");
        // input_id: bustype=BUS_VIRTUAL(0x06), vendor, product, version
        setup[80 + 0..80 + 2].copy_from_slice(&0x06u16.to_le_bytes());
        dev.write_all(&setup)?;
        let rc = unsafe { libc::ioctl(fd, UI_DEV_CREATE as libc::c_ulong) };
        if rc < 0 {
            anyhow::bail!("UI_DEV_CREATE failed");
        }
        Ok(())
    }

    impl InputBackend for UinputBackend {
        fn move_rel(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
            self.emit(EV_REL, REL_X, dx)?;
            self.emit(EV_REL, REL_Y, dy)?;
            self.syn()
        }

        fn move_abs(&mut self, _x: i32, _y: i32) -> anyhow::Result<()> {
            // 未注册 ABS 轴；ABS 模式 backlog
            anyhow::bail!("uinput backend: absolute move not supported (backlog)")
        }

        fn button(&mut self, btn: Btn, down: bool) -> anyhow::Result<()> {
            let code = match btn {
                Btn::Left => BTN_LEFT,
                Btn::Right => BTN_RIGHT,
                Btn::Middle => BTN_MIDDLE,
            };
            self.emit(EV_KEY, code, if down { 1 } else { 0 })?;
            self.syn()
        }

        fn tap(&mut self, btn: Btn, count: u8) -> anyhow::Result<()> {
            for i in 0..count.max(1) {
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                self.button(btn, true)?;
                self.button(btn, false)?;
            }
            Ok(())
        }

        fn scroll_px(&mut self, dx: i32, dy: i32) -> anyhow::Result<()> {
            // 语义：dy>0 = 视图下移。REL_WHEEL 正值 = 上滚 → 垂直取反；REL_HWHEEL 正值 = 右移。
            self.emit(EV_REL, REL_WHEEL_HI_RES, -dy)?;
            self.emit(EV_REL, REL_HWHEEL_HI_RES, dx)?;
            // 离散刻度（老内核/旧应用只认 REL_WHEEL）
            self.scroll_acc_y += dy as f32;
            self.scroll_acc_x += dx as f32;
            let steps_y = (self.scroll_acc_y / SCROLL_PX_PER_STEP).trunc();
            let steps_x = (self.scroll_acc_x / SCROLL_PX_PER_STEP).trunc();
            self.scroll_acc_y -= steps_y * SCROLL_PX_PER_STEP;
            self.scroll_acc_x -= steps_x * SCROLL_PX_PER_STEP;
            if steps_y != 0.0 {
                self.emit(EV_REL, REL_WHEEL, -(steps_y as i32))?;
            }
            if steps_x != 0.0 {
                self.emit(EV_REL, REL_HWHEEL, steps_x as i32)?;
            }
            self.syn()
        }

        fn pinch(&mut self, dscale: i32, _dx: i32, _dy: i32) -> anyhow::Result<()> {
            self.key(Key::Ctrl, true)?;
            self.scroll_px(0, dscale)?;
            self.key(Key::Ctrl, false)?;
            Ok(())
        }

        fn key(&mut self, k: Key, down: bool) -> anyhow::Result<()> {
            let Some(code) = keycode_of_key(k) else {
                anyhow::bail!("no input-event code for {k:?}");
            };
            self.emit(EV_KEY, code, if down { 1 } else { 0 })?;
            self.syn()
        }

        fn text(&mut self, _s: &str) -> anyhow::Result<()> {
            // uinput 无法直接注入 Unicode（需每字符 keymap），v1 不支持
            anyhow::bail!("uinput backend: unicode text not supported on this backend")
        }

        fn media(&mut self, m: MediaKey) -> anyhow::Result<()> {
            let code = keycode_of_media(m);
            self.emit(EV_KEY, code, 1)?;
            self.syn()?;
            self.emit(EV_KEY, code, 0)?;
            self.syn()
        }

        fn screen_size(&self) -> (i32, i32) {
            (0, 0)
        }

        fn trusted(&self) -> bool {
            true
        }
    }
}

/// 运行时选择：X11 → uinput → stub（全部失败时每个操作报错）。
pub fn create_backend() -> Box<dyn InputBackend> {
    if std::env::var_os("DISPLAY").is_some() {
        match x11::X11Backend::connect() {
            Ok(b) => {
                tracing::info!("input backend: X11 (XTEST)");
                return Box::new(b);
            }
            Err(e) => tracing::warn!("X11 connect failed, fallback to uinput: {e}"),
        }
    } else {
        tracing::info!("no DISPLAY, trying uinput (Wayland)");
    }
    match uinput::UinputBackend::open() {
        Ok(b) => {
            tracing::info!("input backend: uinput（需 input 组权限；Wayland 实验性）");
            return Box::new(b);
        }
        Err(e) => tracing::warn!("uinput open failed: {e}"),
    }
    tracing::warn!("no input backend available; injections will error");
    Box::new(super::stub::StubBackend)
}
