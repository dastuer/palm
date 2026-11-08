//! WS 会话层（§6.1 ws.rs）：状态机、心跳、单会话互斥、限流、指针事件分发。
//!
//! 会话状态机（§4.4）：
//! 未配对 ──hello──► pending_pair ──pair_ok──► active
//!                     │                          │ 心跳丢失 15s
//!                     └── 超时 60s ◄────────────┘（回到待配对，token 免配）

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{Sink, SinkExt, StreamExt};

use crate::input::{parse_key, parse_media, Btn, Key};
use crate::proto::{self, ClientMsg, PointerEvent};
use crate::security::{
    PairProgress, PairResult, Security, HEARTBEAT_TIMEOUT, MAX_FRAME_LEN, PENDING_PAIR_TIMEOUT,
    RATE_LIMIT_EVENTS,
};

pub struct Hub {
    pub security: Security,
    pub cfg: Mutex<crate::config::Config>,
    pub cfg_path: std::path::PathBuf,
    pub backend: tokio::sync::Mutex<Box<dyn crate::input::InputBackend>>,
    /// 当前 active 控制会话（conn id）。全局只允许一个（§4.4）。
    pub active: tokio::sync::Mutex<Option<u64>>,
    /// active 会话的设备名（菜单栏状态显示用）
    pub active_name: Mutex<Option<String>>,
    pub conn_seq: AtomicU64,
    pub version: &'static str,
    pub os: &'static str,
}

impl Hub {
    pub fn new(
        security: Security,
        cfg: crate::config::Config,
        cfg_path: std::path::PathBuf,
        backend: Box<dyn crate::input::InputBackend>,
        version: &'static str,
        os: &'static str,
    ) -> Self {
        Self {
            security,
            cfg: Mutex::new(cfg),
            cfg_path,
            backend: tokio::sync::Mutex::new(backend),
            active: tokio::sync::Mutex::new(None),
            active_name: Mutex::new(None),
            conn_seq: AtomicU64::new(1),
            version,
            os,
        }
    }

    pub fn next_conn_id(&self) -> u64 {
        self.conn_seq.fetch_add(1, Ordering::Relaxed)
    }
}

/// 每连接限流器：1s 滑动窗口内最多 RATE_LIMIT_EVENTS 个事件帧。
struct RateLimiter {
    window_start: Instant,
    count: u32,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            count: 0,
        }
    }
    fn allow(&mut self) -> bool {
        if self.window_start.elapsed() > Duration::from_secs(1) {
            self.window_start = Instant::now();
            self.count = 0;
        }
        self.count += 1;
        self.count <= RATE_LIMIT_EVENTS
    }
}

/// HTTP → WS 升级入口。
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(hub): State<Arc<Hub>>,
) -> Response {
    let conn_id = hub.next_conn_id();
    ws.on_upgrade(move |socket| run_session(socket, hub, conn_id))
}

enum Session {
    /// 已 hello、未配对（pending_pair）
    Pending(PairProgress, Instant),
    /// 已配对、可注入（active）
    Active,
}

pub async fn run_session(socket: WebSocket, hub: Arc<Hub>, conn_id: u64) {
    let (mut tx, mut rx) = socket.split();
    let mut session: Option<Session> = None;
    let mut rate = RateLimiter::new();
    let mut last_frame = Instant::now();
    let mut not_trusted_warned = false;
    let mut peer_ua = String::new();
    let mut peer_device_id: Option<String> = None;

    tracing::info!(conn = conn_id, "ws connected");

    let mut watchdog = tokio::time::interval(Duration::from_secs(1));
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let reason = {
        // 断开前要发给客户端的最后一条 JSON（err 等），在优雅关闭阶段发送
        let mut close_msg: Option<serde_json::Value> = None;
        let reason = loop {
            tokio::select! {
                biased;
                msg = rx.next() => {
                    let Some(msg) = msg else {
                        break "client closed".to_string();
                    };
                    last_frame = Instant::now();
                    match msg {
                        Ok(Message::Text(text)) => {
                            if text.len() > MAX_FRAME_LEN {
                                break "frame too large".into();
                            }
                            if !rate.allow() {
                                close_msg = Some(serde_json::json!({"t":"err","e":"rate_limited"}));
                                break "rate limited".into();
                            }
                            let v: serde_json::Value = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(e) => {
                                    send(&mut tx, serde_json::json!({"t":"err","e":"bad_json","m":e.to_string()})).await;
                                    continue;
                                }
                            };
                            match proto::parse_client(&v) {
                                Ok(msg) => {
                                    if let Some(close) = handle_client_msg(
                                        &hub, &mut tx, &mut session, conn_id,
                                        &mut not_trusted_warned, &mut peer_ua,
                                        &mut peer_device_id, msg,
                                        &mut close_msg,
                                    ).await {
                                        break close;
                                    }
                                }
                                Err(e) => {
                                    send(&mut tx, serde_json::json!({"t":"err","e":e.to_string()})).await;
                                }
                            }
                        }
                        Ok(Message::Binary(bytes)) => {
                            if bytes.len() > MAX_FRAME_LEN {
                                break "frame too large".into();
                            }
                            if !rate.allow() {
                                close_msg = Some(serde_json::json!({"t":"err","e":"rate_limited"}));
                                break "rate limited".into();
                            }
                            if !matches!(session, Some(Session::Active)) {
                                // 未配对连接发来的指针帧直接忽略（不响应，防止探测）
                                continue;
                            }
                            match PointerEvent::parse(&bytes) {
                                Ok(ev) => {
                                    if let Some(close) = dispatch_pointer(&hub, ev, &mut not_trusted_warned, &mut tx).await {
                                        break close;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(conn = conn_id, "bad pointer frame: {e}");
                                }
                            }
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = tx.send(Message::Pong(p)).await;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::debug!(conn = conn_id, "ws error: {e}");
                            break "ws error".into();
                        }
                    }
                }
                _ = watchdog.tick() => {
                    match &session {
                        Some(Session::Pending(_, since))
                            if since.elapsed() > PENDING_PAIR_TIMEOUT =>
                        {
                            close_msg = Some(serde_json::json!({"t":"err","e":"pair_timeout"}));
                            break "pending pair timeout".into();
                        }
                        Some(Session::Active)
                            if last_frame.elapsed() > HEARTBEAT_TIMEOUT =>
                        {
                            break "heartbeat timeout".into();
                        }
                        _ => {}
                    }
                }
            }
        };
        // 优雅关闭：发最后一条消息 + Close 帧，并等客户端回 Close（≤1s），
        // 确保错误帧确实刷出到 TCP（否则 drop 时可能丢帧，客户端只看到 1006）。
        if let Some(v) = close_msg {
            match tx.send(Message::Text(v.to_string().into())).await {
                Ok(()) => tracing::debug!(conn = conn_id, "close msg flushed"),
                Err(e) => tracing::debug!(conn = conn_id, "close msg send failed: {e}"),
            }
        }
        match tx.send(Message::Close(None)).await {
            Ok(()) => tracing::debug!(conn = conn_id, "close frame flushed"),
            Err(e) => tracing::debug!(conn = conn_id, "close frame send failed: {e}"),
        }
        let _ = tokio::time::timeout(Duration::from_millis(1000), async {
            while let Some(Ok(m)) = rx.next().await {
                if matches!(m, Message::Close(_)) {
                    break;
                }
            }
        })
        .await;
        reason
    };

    // 释放 active 会话
    let mut active = hub.active.lock().await;
    if *active == Some(conn_id) {
        *active = None;
        *hub.active_name.lock().unwrap() = None;
        tracing::info!(conn = conn_id, "released active session");
    }
    tracing::info!(conn = conn_id, "ws closed: {reason}");
}

async fn send(tx: &mut (impl Sink<Message> + Unpin), v: serde_json::Value) {
    let _ = tx.send(Message::Text(v.to_string().into())).await;
}

/// 返回 Some(reason) 表示要关闭连接。
async fn handle_client_msg(
    hub: &Arc<Hub>,
    tx: &mut (impl Sink<Message> + Unpin),
    session: &mut Option<Session>,
    conn_id: u64,
    not_trusted_warned: &mut bool,
    peer_ua: &mut String,
    peer_device_id: &mut Option<String>,
    msg: ClientMsg,
    close_msg: &mut Option<serde_json::Value>,
) -> Option<String> {
    match msg {
        ClientMsg::Hello(h) => {
            *peer_ua = h.ua.clone();
            *peer_device_id = h.device_id.clone();
            tracing::info!(conn = conn_id, "hello: {} {}x{} dpr={} touch={}", h.ua, h.w, h.h, h.dpr, h.touch);
            let cfg = hub.cfg.lock().unwrap().clone();
            let trusted = hub.backend.lock().await.trusted();
            *session = Some(Session::Pending(PairProgress::default(), Instant::now()));
            send(
                tx,
                serde_json::json!({
                    "t": "welcome",
                    "server": hub.version,
                    "os": hub.os,
                    "screens": [0],
                    "state": "pending_pair",
                    "trusted": trusted,
                    "cfg": {
                        "sens": cfg.input.sens,
                        "natural": cfg.input.natural,
                        "scroll_speed": cfg.input.scroll_speed,
                        "tap_enabled": true,
                        "g3_up": cfg.gesture.g3_up,
                        "g3_down": cfg.gesture.g3_down,
                        "g3_left": cfg.gesture.g3_left,
                        "g3_right": cfg.gesture.g3_right,
                    },
                }),
            )
            .await;
            if !trusted {
                tracing::warn!("backend has no input permission yet; injection will be dropped by OS");
            }
            None
        }
        ClientMsg::Pair(p) => {
            let granted = {
                let mut cfg = hub.cfg.lock().unwrap();
                let progress = match session {
                    Some(Session::Pending(prog, _)) => prog.clone(),
                    _ => PairProgress::default(),
                };
                let mut progress = progress;
                let r = hub.security.pair(
                    &mut cfg,
                    &p.mode,
                    p.token.as_deref(),
                    p.pin.as_deref(),
                    p.cred.as_deref(),
                    &mut progress,
                    peer_ua,
                    peer_device_id.as_deref(),
                );
                if let Some(Session::Pending(prog, _)) = session {
                    *prog = progress;
                }
                r
            };
            match granted {
                PairResult::Granted { device } => {
                    // 单会话互斥
                    {
                        let mut active = hub.active.lock().await;
                        if active.is_some() && *active != Some(conn_id) {
                            *close_msg = Some(serde_json::json!({"t":"err","e":"busy","m":"another device is controlling"}));
                            return Some("second controller rejected".into());
                        }
                        *active = Some(conn_id);
                        *hub.active_name.lock().unwrap() = Some(device.name.clone());
                    }
                    *session = Some(Session::Active);
                    tracing::info!(conn = conn_id, "paired device={:?} mode={}", device.name, p.mode);
                    // cred 随 pair_ok 下发（§7：只存手机端 + 服务端设备表）。
                    // 客户端凭它免配重连 —— 缺了它每次重连都要重新扫码配对。
                    send(
                        tx,
                        serde_json::json!({"t":"pair_ok","name":device.name,"cred":device.cred}),
                    )
                    .await;
                }
                PairResult::NeedPin => {
                    send(tx, serde_json::json!({"t":"pair_need_pin"})).await;
                }
                PairResult::Rejected(e) => {
                    tracing::info!(conn = conn_id, "pair rejected: {e}");
                    send(tx, serde_json::json!({"t":"pair_err","e":e})).await;
                }
            }
            None
        }
        ClientMsg::Cfg(patch) => {
            {
                let mut cfg = hub.cfg.lock().unwrap();
                if let Some(s) = patch.sens {
                    cfg.input.sens = s.clamp(0.3, 3.0);
                }
                if let Some(n) = patch.natural {
                    cfg.input.natural = n;
                }
                if let Some(s) = patch.scroll_speed {
                    cfg.input.scroll_speed = s.clamp(0.3, 3.0);
                }
                if let Some(v) = patch.g3_up {
                    cfg.gesture.g3_up = v;
                }
                if let Some(v) = patch.g3_down {
                    cfg.gesture.g3_down = v;
                }
                if let Some(v) = patch.g3_left {
                    cfg.gesture.g3_left = v;
                }
                if let Some(v) = patch.g3_right {
                    cfg.gesture.g3_right = v;
                }
                if let Err(e) = cfg.save(&hub.cfg_path) {
                    tracing::warn!("persist config failed: {e}");
                }
            }
            send(tx, serde_json::json!({"t":"cfg_ok"})).await;
            None
        }
        ClientMsg::Ping { ts } => {
            send(tx, serde_json::json!({"t":"pong","ts":ts})).await;
            None
        }
        // —— 以下均为控制消息，未配对一律拒绝 ——
        msg if !matches!(session, Some(Session::Active)) => {
            send(tx, serde_json::json!({"t":"err","e":"not_paired"})).await;
            if matches!(msg, ClientMsg::Key { .. } | ClientMsg::TapKey { .. } | ClientMsg::Combo { .. } | ClientMsg::Text { .. } | ClientMsg::Media { .. } | ClientMsg::Backspace { .. }) {
                // 未配对就发控制帧：可视为异常客户端
                *close_msg = Some(serde_json::json!({"t":"err","e":"not_paired"}));
                return Some("control frame before pair".into());
            }
            None
        }
        ClientMsg::Key { k, d } => {
            match parse_key(&k) {
                Some(key) => inject(&hub, |b| b.key(key, d), not_trusted_warned, tx).await,
                None => send(tx, serde_json::json!({"t":"err","e":"bad_key"})).await,
            }
            None
        }
        ClientMsg::TapKey { k } => {
            match parse_key(&k) {
                Some(key) => {
                    inject(&hub, |b| {
                        b.key(key, true)?;
                        b.key(key, false)
                    }, not_trusted_warned, tx)
                    .await
                }
                None => send(tx, serde_json::json!({"t":"err","e":"bad_key"})).await,
            }
            None
        }
        ClientMsg::Combo { ks } => {
            let keys: Vec<_> = ks.iter().filter_map(|k| parse_key(k)).collect();
            if keys.len() != ks.len() || keys.is_empty() || ks.len() > 5 {
                send(tx, serde_json::json!({"t":"err","e":"bad_combo"})).await;
                return None;
            }
            inject(&hub, |b| {
                for k in &keys {
                    b.key(*k, true)?;
                }
                for k in keys.iter().rev() {
                    b.key(*k, false)?;
                }
                Ok(())
            }, not_trusted_warned, tx)
            .await;
            None
        }
        ClientMsg::Text { s } => {
            if s.chars().count() > 500 {
                send(tx, serde_json::json!({"t":"err","e":"text_too_long"})).await;
                return None;
            }
            inject(&hub, |b| b.text(&s), not_trusted_warned, tx).await;
            None
        }
        ClientMsg::Backspace { n } => {
            inject(&hub, |b| {
                for _ in 0..n {
                    b.key(Key::Backspace, true)?;
                    b.key(Key::Backspace, false)?;
                }
                Ok(())
            }, not_trusted_warned, tx)
            .await;
            None
        }
        ClientMsg::Media { k } => {
            match parse_media(&k) {
                Some(m) => inject(&hub, |b| b.media(m), not_trusted_warned, tx).await,
                None => send(tx, serde_json::json!({"t":"err","e":"bad_media"})).await,
            }
            None
        }
    }
}

/// 指针事件 → 注入。返回 Some 表示关闭连接。
async fn dispatch_pointer(
    hub: &Arc<Hub>,
    ev: PointerEvent,
    not_trusted_warned: &mut bool,
    tx: &mut (impl Sink<Message> + Unpin),
) -> Option<String> {
    let (natural, speed) = {
        let cfg = hub.cfg.lock().unwrap();
        (cfg.input.natural, cfg.input.scroll_speed)
    };
    let r = match ev {
        PointerEvent::Move { dx, dy } => {
            hub.backend
                .lock()
                .await
                .move_rel(dx as i32, dy as i32)
        }
        PointerEvent::Scroll { dx, dy } => {
            // 前端发"手指位移"语义（dy>0 = 手指下移）。
            // natural（mac 默认，内容跟随手指）：手指下移 → 内容下移 → 视图上移 → 取反注入；
            // 传统滚轮方向：手指下移 → 视图下移 → 同向注入。
            // scroll_px 约定：dy>0 = 视图向文档下方滚（scrollY 增加）。
            let (dx, dy) = if natural { (-dx, -dy) } else { (dx, dy) };
            let dx = (dx as f32 * speed) as i32;
            let dy = (dy as f32 * speed) as i32;
            hub.backend.lock().await.scroll_px(dx, dy)
        }
        PointerEvent::Button { btn, down } => match Btn::from_u8(btn) {
            Some(b) => hub.backend.lock().await.button(b, down),
            None => {
                send(tx, serde_json::json!({"t":"err","e":"bad_btn"})).await;
                return None;
            }
        },
        PointerEvent::Tap { btn, count } => match Btn::from_u8(btn) {
            Some(b) => hub.backend.lock().await.tap(b, count.max(1)),
            None => {
                send(tx, serde_json::json!({"t":"err","e":"bad_btn"})).await;
                return None;
            }
        },
        PointerEvent::Pinch { dscale, dx, dy } => {
            hub.backend.lock().await.pinch(dscale as i32, dx as i32, dy as i32)
        }
        PointerEvent::Gesture { id } => {
            let action = {
                let cfg = hub.cfg.lock().unwrap();
                match id {
                    proto::gesture::UP => cfg.gesture.g3_up.clone(),
                    proto::gesture::DOWN => cfg.gesture.g3_down.clone(),
                    proto::gesture::LEFT => cfg.gesture.g3_left.clone(),
                    proto::gesture::RIGHT => cfg.gesture.g3_right.clone(),
                    _ => "none".to_string(),
                }
            };
            let keys: Vec<Key> = match action.as_str() {
                "mission_control" => vec![Key::Ctrl, Key::Up],
                "app_expose" => vec![Key::Ctrl, Key::Down],
                "space_left" => vec![Key::Ctrl, Key::Left],
                "space_right" => vec![Key::Ctrl, Key::Right],
                _ => vec![],
            };
            if !keys.is_empty() {
                inject(&hub, |b| {
                    for k in &keys {
                        b.key(*k, true)?;
                    }
                    for k in keys.iter().rev() {
                        b.key(*k, false)?;
                    }
                    Ok(())
                }, not_trusted_warned, tx)
                .await;
            }
            Ok(())
        }
        PointerEvent::Abs { x, y } => hub.backend.lock().await.move_abs(x as i32, y as i32),
    };
    if let Err(e) = r {
        tracing::warn!("inject failed: {e}");
    }
    None
}

async fn inject(
    hub: &Arc<Hub>,
    f: impl FnOnce(&mut dyn crate::input::InputBackend) -> anyhow::Result<()>,
    not_trusted_warned: &mut bool,
    tx: &mut (impl Sink<Message> + Unpin),
) {
    let mut b = hub.backend.lock().await;
    let trusted = b.trusted();
    if !trusted {
        if !*not_trusted_warned {
            *not_trusted_warned = true;
            send(tx, serde_json::json!({"t":"err","e":"no_accessibility","m":"server lacks input permission"})).await;
            tracing::warn!("injection attempted without accessibility permission; sending no_accessibility hint");
        }
        return;
    }
    if let Err(e) = f(b.as_mut()) {
        tracing::warn!("inject failed: {e}");
    }
}
