//! Palm 协议层：JSON 控制帧 + 二进制指针帧的解析与构造。
//!
//! 参考 docs/DESIGN.md §4。text 帧 = JSON（低频控制），binary 帧 = 定长指针事件（高频）。

use serde::{Deserialize, Serialize};

/// 二进制指针帧类型字节（§4.3）。
pub mod frame {
    pub const MOVE: u8 = 0x01;
    pub const SCROLL: u8 = 0x02;
    pub const BUTTON: u8 = 0x03;
    pub const TAP: u8 = 0x04;
    pub const PINCH: u8 = 0x05;
    pub const GESTURE: u8 = 0x06;
    pub const ABS: u8 = 0x07;
}

/// 三指手势 id（§4.3 GESTURE）。
pub mod gesture {
    pub const UP: u8 = 0;
    pub const DOWN: u8 = 1;
    pub const LEFT: u8 = 2;
    pub const RIGHT: u8 = 3;
}

/// 鼠标按钮编号（§4.3 BUTTON/TAP，X1/X2 保留协议兼容）。
#[allow(dead_code)]
pub mod btn {
    pub const LEFT: u8 = 0;
    pub const RIGHT: u8 = 1;
    pub const MIDDLE: u8 = 2;
    pub const X1: u8 = 3;
    pub const X2: u8 = 4;
}

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("binary frame too short: {0} bytes")]
    TooShort(usize),
    #[error("unknown binary frame type: 0x{0:02x}")]
    UnknownType(u8),
    #[error("unknown message type: {0}")]
    UnknownMsg(String),
    #[error("bad json field: {0}")]
    BadField(String),
    #[error("bad json: {0}")]
    Json(#[from] serde_json::Error),
}

/// 解析后的二进制指针事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvent {
    /// MOVE 0x01：[i16 dx][i16 dy]，已含灵敏度/加速的最终位移
    Move { dx: i16, dy: i16 },
    /// SCROLL 0x02：[i16 dx][i16 dy]，手指位移方向的自然增量（含惯性帧）
    Scroll { dx: i16, dy: i16 },
    /// BUTTON 0x03：[u8 btn][u8 down]
    Button { btn: u8, down: bool },
    /// TAP 0x04：[u8 btn][u8 count] 合成点击
    Tap { btn: u8, count: u8 },
    /// PINCH 0x05：[i16 dscale][i16 dx][i16 dy]（dscale 单位 px，正=张开）
    Pinch { dscale: i16, dx: i16, dy: i16 },
    /// GESTURE 0x06：[u8 id]
    Gesture { id: u8 },
    /// ABS 0x07：[i16 x][i16 y]（虚拟桌面归一化 ×65535，backlog）
    Abs { x: i16, y: i16 },
}

impl PointerEvent {
    pub fn parse(data: &[u8]) -> Result<Self, ProtoError> {
        if data.is_empty() {
            return Err(ProtoError::TooShort(0));
        }
        let le_i16 = |off: usize| i16::from_le_bytes([data[off], data[off + 1]]);
        Ok(match data[0] {
            frame::MOVE if data.len() >= 5 => PointerEvent::Move {
                dx: le_i16(1),
                dy: le_i16(3),
            },
            frame::SCROLL if data.len() >= 5 => PointerEvent::Scroll {
                dx: le_i16(1),
                dy: le_i16(3),
            },
            frame::BUTTON if data.len() >= 3 => PointerEvent::Button {
                btn: data[1],
                down: data[2] != 0,
            },
            frame::TAP if data.len() >= 3 => PointerEvent::Tap {
                btn: data[1],
                count: data[2],
            },
            frame::PINCH if data.len() >= 7 => PointerEvent::Pinch {
                dscale: le_i16(1),
                dx: le_i16(3),
                dy: le_i16(5),
            },
            frame::GESTURE if data.len() >= 2 => PointerEvent::Gesture { id: data[1] },
            frame::ABS if data.len() >= 5 => PointerEvent::Abs {
                x: le_i16(1),
                y: le_i16(3),
            },
            frame::MOVE
            | frame::SCROLL
            | frame::BUTTON
            | frame::TAP
            | frame::PINCH
            | frame::GESTURE
            | frame::ABS => return Err(ProtoError::TooShort(data.len())),
            t => return Err(ProtoError::UnknownType(t)),
        })
    }
}

// ---------------------------------------------------------------------------
// JSON 控制消息（§4.2）
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Hello {
    #[allow(dead_code)] // 协议版本字段，向前兼容保留
    pub v: u8,
    #[serde(default)]
    pub ua: String,
    #[serde(default)]
    pub w: u16,
    #[serde(default)]
    pub h: u16,
    #[serde(default)]
    pub dpr: f32,
    #[serde(default)]
    pub touch: bool,
    /// 客户端稳定设备标识（设备去重；缺省 = 旧客户端，不去重）
    #[serde(default)]
    pub device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PairMsg {
    pub mode: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub pin: Option<String>,
    #[serde(default)]
    pub cred: Option<String>,
}

/// 客户端同步来的设置补丁（可只带部分字段）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CfgPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sens: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_enabled: Option<bool>,
    /// 三指动作映射：mission_control | app_expose | space_left | space_right | none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g3_up: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g3_down: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g3_left: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g3_right: Option<String>,
}

/// 客户端 → 服务端的一条控制消息（手动按 "t" 分发，避免 serde tag+flatten 冲突）。
#[derive(Debug)]
pub enum ClientMsg {
    Hello(Hello),
    Pair(PairMsg),
    Key { k: String, d: bool },
    TapKey { k: String },
    Combo { ks: Vec<String> },
    Text { s: String },
    Backspace { n: u32 },
    Media { k: String },
    Cfg(CfgPatch),
    Ping { ts: u64 },
}

pub fn parse_client(v: &serde_json::Value) -> Result<ClientMsg, ProtoError> {
    let t = v
        .get("t")
        .and_then(|x| x.as_str())
        .ok_or_else(|| ProtoError::BadField("missing t".into()))?;
    let from = |guard: &str| -> Result<&serde_json::Value, ProtoError> {
        if t != guard {
            return Err(ProtoError::BadField(format!("t={t}, want {guard}")));
        }
        Ok(v)
    };
    let get = |v: &serde_json::Value, k: &str| v.get(k).cloned().unwrap_or_default();
    Ok(match t {
        "hello" => ClientMsg::Hello(serde_json::from_value(from("hello")?.clone())?),
        "pair" => ClientMsg::Pair(serde_json::from_value(from("pair")?.clone())?),
        "key" => {
            let v = from("key")?;
            ClientMsg::Key {
                k: str_of(&get(v, "k"))?,
                d: get(v, "d").as_i64().map(|d| d != 0).unwrap_or(false),
            }
        }
        "tap" => ClientMsg::TapKey {
            k: str_of(&get(from("tap")?, "k"))?,
        },
        "combo" => {
            let v = from("combo")?;
            let ks = get(v, "ks")
                .as_array()
                .ok_or_else(|| ProtoError::BadField("ks".into()))?
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect();
            ClientMsg::Combo { ks }
        }
        "text" => ClientMsg::Text {
            s: str_of(&get(from("text")?, "s"))?,
        },
        "backspace" => ClientMsg::Backspace {
            n: get(from("backspace")?, "n").as_u64().unwrap_or(1).min(100) as u32,
        },
        "media" => ClientMsg::Media {
            k: str_of(&get(from("media")?, "k"))?,
        },
        "cfg" => ClientMsg::Cfg(serde_json::from_value(from("cfg")?.clone())?),
        "ping" => ClientMsg::Ping {
            ts: get(from("ping")?, "ts").as_u64().unwrap_or(0),
        },
        other => return Err(ProtoError::UnknownMsg(other.to_owned())),
    })
}

fn str_of(v: &serde_json::Value) -> Result<String, ProtoError> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| ProtoError::BadField("expected string".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_binary_frames() {
        assert_eq!(
            PointerEvent::parse(&[0x01, 0x0a, 0x00, 0xf6, 0xff]).unwrap(),
            PointerEvent::Move { dx: 10, dy: -10 }
        );
        assert_eq!(
            PointerEvent::parse(&[0x02, 0x00, 0x00, 0x05, 0x00]).unwrap(),
            PointerEvent::Scroll { dx: 0, dy: 5 }
        );
        assert_eq!(
            PointerEvent::parse(&[0x03, 0x01, 0x01]).unwrap(),
            PointerEvent::Button { btn: 1, down: true }
        );
        assert_eq!(
            PointerEvent::parse(&[0x04, 0x00, 0x02]).unwrap(),
            PointerEvent::Tap { btn: 0, count: 2 }
        );
        assert_eq!(
            PointerEvent::parse(&[0x05, 0x0f, 0x00, 0, 0, 0, 0]).unwrap(),
            PointerEvent::Pinch { dscale: 15, dx: 0, dy: 0 }
        );
        assert_eq!(
            PointerEvent::parse(&[0x06, 0x02]).unwrap(),
            PointerEvent::Gesture { id: 2 }
        );
        assert!(PointerEvent::parse(&[0x01, 0x01]).is_err());
        assert!(PointerEvent::parse(&[0x09]).is_err());
    }

    #[test]
    fn parse_json_msgs() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"t":"key","k":"cmd","d":1}"#).unwrap();
        assert!(matches!(parse_client(&v).unwrap(), ClientMsg::Key { k, d: true } if k == "cmd"));
        let v: serde_json::Value =
            serde_json::from_str(r#"{"t":"cfg","sens":1.2,"natural":true}"#).unwrap();
        match parse_client(&v).unwrap() {
            ClientMsg::Cfg(c) => {
                assert_eq!(c.sens, Some(1.2));
                assert_eq!(c.natural, Some(true));
                assert_eq!(c.scroll_speed, None);
            }
            _ => panic!(),
        }
        let v: serde_json::Value = serde_json::from_str(r#"{"t":"ping","ts":1699}"#).unwrap();
        assert!(matches!(parse_client(&v).unwrap(), ClientMsg::Ping { ts: 1699 }));
        let v: serde_json::Value = serde_json::from_str(r#"{"t":"wat"}"#).unwrap();
        assert!(parse_client(&v).is_err());
    }
}
