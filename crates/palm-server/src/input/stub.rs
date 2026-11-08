//! M4 前的占位后端：任何注入操作都返回"未实现"。
//! 保持协议与会话层在非 macOS 平台可编译、可联调（不含注入）。

use super::{Btn, InputBackend, Key, MediaKey};

pub struct StubBackend;

const UNSUPPORTED: &str = "input injection is not implemented on this platform yet (planned for M4)";

impl InputBackend for StubBackend {
    fn move_rel(&mut self, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn move_abs(&mut self, _x: i32, _y: i32) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn button(&mut self, _btn: Btn, _down: bool) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn tap(&mut self, _btn: Btn, _count: u8) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn scroll_px(&mut self, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn pinch(&mut self, _dscale: i32, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn key(&mut self, _k: Key, _down: bool) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn text(&mut self, _s: &str) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn media(&mut self, _m: MediaKey) -> anyhow::Result<()> {
        anyhow::bail!(UNSUPPORTED)
    }
    fn screen_size(&self) -> (i32, i32) {
        (0, 0)
    }
    fn trusted(&self) -> bool {
        true
    }
}
