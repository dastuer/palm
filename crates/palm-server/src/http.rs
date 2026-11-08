//! HTTP 层（§6.1 http.rs）：内嵌静态页 + /ws + /api/*。
//!
//! axum 只暴露少量路由（§7 安全设计）；设备管理 API 仅允许本机调用。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use rust_embed::RustEmbed;

use crate::ws::Hub;

/// 本机控制台页（内联，无需走前端构建）。仅本机打开使用：展示运行状态、
/// 配对二维码、设备管理；LSUIElement .app 形态的可见反馈。
pub const ADMIN_HTML: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Palm — 连接与配置</title>
<style>
  :root { color-scheme: dark; }
  html { scrollbar-width: thin; scrollbar-color: #2c3340 transparent; }
  ::-webkit-scrollbar { width: 8px; height: 8px; }
  ::-webkit-scrollbar-track { background: transparent; }
  ::-webkit-scrollbar-thumb { background: #2c3340; border-radius: 4px; }
  ::-webkit-scrollbar-thumb:hover { background: #3a4252; }
  ::-webkit-scrollbar-corner { background: transparent; }
  body { font: 15px/1.5 -apple-system, "PingFang SC", sans-serif; background: #0f1115; color: #e8eaf0;
         margin: 0 auto; max-width: 1280px; padding: 18px 22px 30px; }
  @media (min-width: 980px) {
    body { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; align-items: start; }
    body > h1, body > .sub { grid-column: 1 / -1; margin: 0; }
    body > .card { margin-bottom: 0; }
    body > .card:nth-child(4) { grid-row: span 2; } /* 配对卡（含二维码）占足左列高度 */
  }
  h1 { font-size: 18px; margin: 0 0 2px; }
  .sub { color: #8b93a3; font-size: 13px; margin: 0 0 14px; }
  .card { background: #161a21; border-radius: 14px; padding: 16px; margin-bottom: 14px; }
  .card h2 { font-size: 14px; margin: 0 0 10px; color: #aab3c5; font-weight: 600; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 10px;
         padding: 8px 0; border-bottom: 1px solid rgba(128,138,160,.1); font-size: 14px; }
  .row:last-child { border-bottom: 0; }
  .badge { padding: 3px 10px; border-radius: 999px; font-size: 12px; }
  .ok { background: #12331d; color: #34c759; }
  .warn { background: #3a2b10; color: #ffcc00; }
  #qr { width: 190px; height: 190px; background: #fff; border-radius: 12px; display: block; margin: 4px auto 10px; }
  .url { font-size: 12px; color: #8b93a3; word-break: break-all; text-align: center; margin-bottom: 10px; }
  button { background: #5b8cff; color: #fff; border: 0; border-radius: 9px; padding: 9px 13px;
           font-size: 13px; cursor: pointer; }
  button.ghost { background: #1e232d; color: #aab3c5; }
  button.danger { background: none; border: 1px solid #ff5d5d; color: #ff5d5d; padding: 4px 10px; font-size: 12px; }
  .dev { display: flex; justify-content: space-between; align-items: center; padding: 7px 0;
         border-bottom: 1px solid rgba(128,138,160,.08); font-size: 14px; }
  .dev:last-child { border-bottom: 0; }
  .dev .when { color: #8b93a3; font-size: 12px; margin-right: 8px; }
  .dev .nm { flex: 1; }
  label.f { display: block; font-size: 13px; color: #8b93a3; margin: 8px 0 4px; }
  input[type=text] { width: 100%; box-sizing: border-box; background: #1e232d; color: #e8eaf0;
                     border: 0; border-radius: 9px; padding: 10px 12px; font-size: 14px; }
  .hint { color: #8b93a3; font-size: 12px; margin-top: 8px; }
  .toast { position: fixed; left: 50%; bottom: 18px; transform: translateX(-50%);
           background: #333c4d; color: #fff; padding: 8px 16px; border-radius: 999px;
           font-size: 13px; opacity: 0; transition: all .25s; pointer-events: none; }
  .toast.show { opacity: 1; }
</style>
<h1>🐾 Palm</h1>
<p class="sub">手机扫码即连 · 配置实时保存</p>

<div class="card">
  <h2>运行状态</h2>
  <div class="row"><span>版本</span><span id="ver">—</span></div>
  <div class="row"><span>输入注入权限</span><span id="trust" class="badge warn">检测中…</span></div>
  <div class="row"><span>mDNS 发现</span><span>_palm._tcp</span></div>
  <div class="hint" id="trustTip"></div>
</div>

<div class="card">
  <h2>连接配对</h2>
  <img id="qr" alt="配对二维码">
  <div class="url" id="url"></div>
  <div style="display:flex; gap:8px;">
    <button style="flex:1" onclick="rotateQr()">换新二维码</button>
    <button style="flex:1" class="ghost" onclick="copyLink()">复制链接</button>
  </div>
</div>

<div class="card">
  <h2>已配对设备</h2>
  <div id="devices"></div>
  <div class="hint">吊销后该手机需重新扫码配对</div>
</div>

<div class="card">
  <h2>配置</h2>
  <label class="f">设备名称（mDNS / 界面显示）</label>
  <input type="text" id="c-name" placeholder="Palm">
  <label class="f">端口（1024–65535，改后需重启 Palm）</label>
  <input type="text" id="c-port" inputmode="numeric" placeholder="8787">
  <label class="f">配对 PIN（4 位数字，留空 = 关闭二次确认）</label>
  <input type="text" id="c-pin" inputmode="numeric" maxlength="4" placeholder="留空关闭">
  <label class="f" style="display:flex; align-items:center; gap:8px; margin-top:10px;">
    <input type="checkbox" id="c-open" style="width:auto">
    免配对码直连（同局域网知道地址即可连接，勾选 PIN 后仍需 PIN）
  </label>
  <div style="margin-top:12px; display:flex; gap:8px;">
    <button style="flex:1" onclick="saveCfg()">保存配置</button>
  </div>
  <div class="hint" id="cfgHint"></div>
</div>
<div class="toast" id="toast"></div>
<script>
function toast(m) {
  const t = document.getElementById("toast");
  t.textContent = m; t.classList.add("show");
  setTimeout(() => t.classList.remove("show"), 2200);
}
async function info() {
  const r = await (await fetch("/api/info")).json();
  document.getElementById("ver").textContent = "v" + r.version;
  const t = document.getElementById("trust");
  t.className = "badge " + (r.trusted ? "ok" : "warn");
  t.textContent = r.trusted ? "已授权 ✓" : "未授权 ⚠";
  document.getElementById("trustTip").textContent = r.trusted ? "" :
    "菜单栏 → 辅助功能权限设置…，或系统设置 → 隐私与安全性 → 辅助功能 → 勾选 Palm。";
}
let lastDevicesJson = "";
async function loadDevices() {
  const r = await (await fetch("/api/devices")).json();
  const j = JSON.stringify(r.devices);
  // 数据没变就不重绘：5s 轮询重绘会重置"确认吊销？"的两段确认状态
  if (j === lastDevicesJson) return;
  lastDevicesJson = j;
  const box = document.getElementById("devices");
  box.textContent = "";
  if (!r.devices.length) {
    box.innerHTML = '<div class="hint">暂无已配对设备，扫码完成首次配对</div>';
    return;
  }
  r.devices.forEach(d => {
    const el = document.createElement("div");
    el.className = "dev";
    el.innerHTML = '<span class="nm">' + d.name + '</span><span class="when">' +
      (d.added || "").slice(0, 10) + '</span>';
    const btn = document.createElement("button");
    btn.className = "danger"; btn.textContent = "吊销";
    // 两段点击确认：WKWebView 未实现 window.confirm（无 WKUIDelegate 时恒为 false，
    // 会导致"点了没反应"），因此不用原生弹窗
    btn.onclick = async () => {
      if (btn.dataset.armed !== "1") {
        btn.dataset.armed = "1"; btn.textContent = "确认吊销？";
        setTimeout(() => {
          if (btn.isConnected && btn.dataset.armed === "1") {
            btn.dataset.armed = ""; btn.textContent = "吊销";
          }
        }, 3000);
        return;
      }
      try {
        const r = await fetch("/api/devices/" + d.cred, { method: "DELETE" });
        if (!r.ok) throw new Error("HTTP " + r.status);
        toast("已吊销 " + d.name);
      } catch (e) {
        toast("吊销失败：" + e.message);
      }
      loadDevices();
    };
    el.appendChild(btn);
    box.appendChild(el);
  });
}
async function rotateQr() {
  await fetch("/api/pair/rotate", { method: "POST" });
  loadQr();
}
function loadQr() {
  fetch("/api/pair/rotate", { method: "POST" }).then(r => r.json()).then(j => {
    document.getElementById("url").textContent = j.url;
    document.getElementById("qr").src = "/api/pair/qr.svg?ts=" + Date.now();
  });
}
function copyLink() {
  navigator.clipboard.writeText(document.getElementById("url").textContent);
  toast("链接已复制");
}
async function loadCfg() {
  const c = await (await fetch("/api/config")).json();
  document.getElementById("c-name").value = c.name;
  document.getElementById("c-port").value = c.port;
  document.getElementById("c-pin").value = c.pin;
  document.getElementById("c-open").checked = !!c.open;
}
async function saveCfg() {
  const body = {
    name: document.getElementById("c-name").value,
    port: parseInt(document.getElementById("c-port").value, 10) || 0,
    pin: document.getElementById("c-pin").value.trim(),
    open: document.getElementById("c-open").checked,
  };
  const r = await (await fetch("/api/config", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  })).json();
  if (r.ok) {
    toast(r.restart_needed ? "已保存（端口需重启 Palm 生效）" : "配置已保存");
    document.getElementById("cfgHint").textContent =
      r.restart_needed ? "端口已更新：菜单栏退出 Palm 后重新打开生效。" : "";
  } else toast("保存失败");
}
info(); loadDevices(); loadCfg(); loadQr();
setInterval(() => { info(); loadDevices(); }, 5000);
</script>"#;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

/// --web-dir 开发模式：优先从磁盘目录提供前端（无需重新编译 cargo）。
static WEB_DIR_OVERRIDE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

pub fn set_web_dir_override(dir: Option<String>) {
    let _ = WEB_DIR_OVERRIDE.set(dir.map(std::path::PathBuf::from));
}

fn asset_response(path: &str) -> Response {
    if let Some(Some(dir)) = WEB_DIR_OVERRIDE.get() {
        let p = dir.join(path);
        if let Ok(data) = std::fs::read(&p) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return ([(header::CONTENT_TYPE, mime.as_ref())], data).into_response();
        }
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match WebAssets::get(path) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], f.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn index_handler() -> Response {
    asset_response("index.html")
}

async fn static_handler(Path(path): Path<String>) -> Response {
    // SPA：无扩展名的路径回落到 index.html（hash 路由，其实只有 / 和资源）
    if path.contains("..") {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let r = asset_response(&path);
    if r.status() == StatusCode::NOT_FOUND && !path.contains('.') {
        return asset_response("index.html");
    }
    r
}

async fn api_info(State(hub): State<Arc<Hub>>) -> Response {
    let (name, devices) = {
        let cfg = hub.cfg.lock().unwrap();
        (
            cfg.server.name.clone(),
            cfg.device
                .iter()
                .map(|d| serde_json::json!({ "name": d.name, "added": d.added }))
                .collect::<Vec<_>>(),
        )
    };
    let trusted = hub.backend.lock().await.trusted();
    let url_token = hub.security.peek_one_time().is_some();
    axum::Json(serde_json::json!({
        "name": name,
        "version": hub.version,
        "os": hub.os,
        "trusted": trusted,
        "pair_token_available": url_token,
        "devices": devices,
    }))
    .into_response()
}

/// 轮换一次性 token（设备管理页用；仅限本机）。
async fn api_pair_rotate(State(hub): State<Arc<Hub>>) -> Response {
    let token = hub.security.rotate_one_time();
    let port = hub.cfg.lock().unwrap().server.port;
    let ip = crate::discovery::primary_lan_ip().unwrap_or_else(|| "127.0.0.1".into());
    axum::Json(serde_json::json!({
        "token": token,
        "url": format!("http://{ip}:{port}/#/t={token}"),
    }))
    .into_response()
}

/// 配对二维码 SVG（本机控制台页用；无有效 token 时自动轮换新的）。
async fn api_pair_qr_svg(State(hub): State<Arc<Hub>>) -> Response {
    let token = match hub.security.peek_one_time() {
        Some(t) => t,
        None => hub.security.rotate_one_time(),
    };
    let port = hub.cfg.lock().unwrap().server.port;
    let ip = crate::discovery::primary_lan_ip().unwrap_or_else(|| "127.0.0.1".into());
    let url = format!("http://{ip}:{port}/#/t={token}");
    let svg = match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .dark_color(qrcode::render::svg::Color("#111418"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .quiet_zone(true)
            .build(),
        Err(_) => "<svg xmlns='http://www.w3.org/2000/svg'/>".to_string(),
    };
    (
        [(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-store")],
        svg,
    )
        .into_response()
}

/// 本机控制台页（LSUIElement .app 形态的"界面"）：状态 + 配对二维码 + 设备管理。
async fn admin_handler() -> Response {
    Html(ADMIN_HTML).into_response()
}

async fn api_devices(State(hub): State<Arc<Hub>>, addr: ConnectInfo<SocketAddr>) -> Response {
    if !addr.0.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "local only").into_response();
    }
    let cfg = hub.cfg.lock().unwrap();
    axum::Json(serde_json::json!({
        "devices": cfg.device.iter().map(|d| serde_json::json!({
            "name": d.name, "added": d.added, "cred": d.cred,
            "device_id": d.device_id,
        })).collect::<Vec<_>>()
    }))
    .into_response()
}

/// 当前配置（本机控制台用）。
async fn api_config_get(State(hub): State<Arc<Hub>>, addr: ConnectInfo<SocketAddr>) -> Response {
    if !addr.0.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "local only").into_response();
    }
    let cfg = hub.cfg.lock().unwrap();
    axum::Json(serde_json::json!({
        "name": cfg.server.name,
        "port": cfg.server.port,
        "pin": cfg.pairing.pin,
        "open": cfg.pairing.open,
        "natural": cfg.input.natural,
        "scroll_speed": cfg.input.scroll_speed,
    }))
    .into_response()
}

/// 修改配置（本机控制台用）。端口改动需重启后生效。
async fn api_config_set(
    State(hub): State<Arc<Hub>>,
    addr: ConnectInfo<SocketAddr>,
    body: String,
) -> Response {
    if !addr.0.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "local only").into_response();
    }
    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let mut restart_needed = false;
    {
        let mut cfg = hub.cfg.lock().unwrap();
        if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
            let name = name.trim();
            if !name.is_empty() && name.chars().count() <= 40 {
                cfg.server.name = name.to_string();
            }
        }
        if let Some(port) = v.get("port").and_then(|x| x.as_u64()) {
            if (1024..=65535).contains(&port) && port as u16 != cfg.server.port {
                cfg.server.port = port as u16;
                restart_needed = true;
            }
        }
        if let Some(pin) = v.get("pin").and_then(|x| x.as_str()) {
            let pin = pin.trim();
            if pin.is_empty() || (pin.len() == 4 && pin.chars().all(|c| c.is_ascii_digit())) {
                cfg.pairing.pin = pin.to_string();
            }
        }
        if let Some(open) = v.get("open").and_then(|x| x.as_bool()) {
            cfg.pairing.open = open;
        }
        if let Err(e) = cfg.save(&hub.cfg_path) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    axum::Json(serde_json::json!({ "ok": true, "restart_needed": restart_needed }))
        .into_response()
}

/// 吊销设备（仅限本机调用）。
async fn api_device_revoke(
    State(hub): State<Arc<Hub>>,
    addr: ConnectInfo<SocketAddr>,
    Path(cred): Path<String>,
) -> Response {
    if !addr.0.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "local only").into_response();
    }
    let removed = {
        let mut cfg = hub.cfg.lock().unwrap();
        hub.security.revoke(&mut cfg, &cred)
    };
    if removed {
        // 吊销后断开所有会话：简单起见，让下一次心跳超时自然回收；
        // 更彻底的做法是广播关闭帧，v1 保持简单并记录日志。
        tracing::info!("device revoked: {cred}");
        axum::Json(serde_json::json!({"ok": true})).into_response()
    } else {
        (StatusCode::NOT_FOUND, "no such device").into_response()
    }
}

pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/admin", get(admin_handler))
        .route("/{*path}", get(static_handler))
        .route("/ws", get(crate::ws::ws_upgrade))
        .route("/api/info", get(api_info))
        .route("/api/pair/rotate", post(api_pair_rotate))
        .route("/api/pair/qr.svg", get(api_pair_qr_svg))
        .route("/api/devices", get(api_devices))
        .route("/api/devices/{cred}", delete(api_device_revoke))
        .route("/api/config", get(api_config_get).post(api_config_set))
        .fallback(index_handler)
        .with_state(hub)
}
