//! Palm server 入口（§6.1 main.rs）。
//!
//! 用法：palm [--port N] [--no-mdns] [--web-dir DIR] [--config PATH]
//! 启动即打印配对 URL + 终端二维码（§6.3 首次运行流程）。

mod config;
mod discovery;
mod http;
mod input;
mod proto;
mod security;
#[cfg(target_os = "macos")]
mod ui;
mod ws;

use std::sync::Arc;

use qrcode::QrCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Args {
    port: Option<u16>,
    no_mdns: bool,
    web_dir: Option<String>,
    config: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        port: None,
        no_mdns: false,
        web_dir: None,
        config: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" | "-p" => args.port = it.next().and_then(|v| v.parse().ok()),
            "--no-mdns" => args.no_mdns = true,
            "--web-dir" => args.web_dir = it.next(),
            "--config" => args.config = it.next(),
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("palm {VERSION}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    args
}

fn print_help() {
    println!(
        "palm {VERSION} — Phone-As-Laptop-Mouse\n\n\
         用法: palm [选项]\n\n\
         选项:\n  \
         -p, --port N      HTTP/WS 端口（默认 8787）\n  \
         --no-mdns         关闭 mDNS 广播\n  \
         --web-dir DIR     从磁盘目录提供前端（开发模式，默认用内嵌产物）\n  \
         --config PATH     配置文件路径（默认 ~/.config/palm/palm.toml）\n  \
         -h, --help        帮助\n  \
         -V, --version     版本"
    );
}

fn print_qr(url: &str) {
    match QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let img = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Dark)
                .light_color(qrcode::render::unicode::Dense1x2::Light)
                .quiet_zone(true)
                .build();
            println!("{img}");
        }
        Err(e) => tracing::warn!("qr render failed: {e}"),
    }
}

fn banner() {
    println!(
        r#"
 ██████╗  █████╗ ██╗     ███╗   ███╗
 ██╔══██╗██╔══██╗██║     ████╗ ████║
 ██████╔╝███████║██║     ██╔████╔██║
 ██╔═══╝ ██╔══██║██║     ██║╚██╔╝██║
 ██║     ██║  ██║███████╗██║ ╚═╝ ██║
 ╚═╝     ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝
  Phone-As-Laptop-Mouse  v{VERSION}
"#
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 子命令：palm qr <text> —— 在终端打印二维码（.app 形态无终端，配对页/排障时可重新展示）
    if std::env::args().nth(1).as_deref() == Some("qr") {
        let text = std::env::args().nth(2).unwrap_or_default();
        if text.is_empty() {
            eprintln!("usage: palm qr <text>");
            std::process::exit(2);
        }
        print_qr(&text);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,palm_server=info".into()),
        )
        .with_target(false)
        .init();

    let args = parse_args();
    banner();
    http::set_web_dir_override(args.web_dir.clone());

    let cfg_path: std::path::PathBuf = args
        .config
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);
    let cfg = config::Config::load_or_create(&cfg_path)?;
    let port = args.port.unwrap_or(cfg.server.port);

    // 输入后端 + 权限检测（§6.3.1）
    let mut backend = input::create_backend();
    if !backend.trusted() {
        eprintln!("⚠️  缺少输入注入权限（macOS: 辅助功能）。");
        eprintln!("   打开 系统设置 → 隐私与安全性 → 辅助功能，勾选运行 palm 的终端 App。");
        eprintln!("   授权后 palm 会自动恢复注入（无需重启）。\n");
        // 异步引导一次深链（不阻塞启动）
        let _ = backend.request_trust();
    }

    let hub = Arc::new(ws::Hub::new(
        security::Security::new(cfg_path.clone()),
        cfg,
        cfg_path.clone(),
        backend,
        VERSION,
        os_name(),
    ));

    let token = hub.security.rotate_one_time();
    let ip = discovery::primary_lan_ip().unwrap_or_else(|| "127.0.0.1".into());
    let url = format!("http://{ip}:{port}/#/t={token}");

    // GUI 模式（.app）：LSEnvironment 注入 PALM_OPEN_ADMIN=1（见 make-app.sh）
    let gui = cfg!(target_os = "macos")
        && std::env::var("PALM_OPEN_ADMIN").as_deref() == Ok("1");

    if gui {
        println!("🐾 菜单栏模式：图标在屏幕右上角菜单栏，点击可打开控制台/退出");
    } else {
        println!("📱 用手机相机扫码，或在手机浏览器打开：\n");
        print_qr(&url);
        println!("   {url}\n");
    }
    println!("   端口 {port} · 配置 {cfg_path:?}");
    println!("   mDNS: {}\n", if args.no_mdns { "off" } else { "_palm._tcp" });

    // mDNS 广播
    let _mdns = if args.no_mdns {
        None
    } else {
        match discovery::advertise(&hub.cfg.lock().unwrap().server.name.clone(), port) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!("mDNS failed (可忽略，二维码直连不受影响): {e}");
                None
            }
        }
    };

    let router = http::router(hub.clone());
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) if gui => {
            // 已有实例在跑（菜单栏图标仍在），本实例退出即可
            anyhow::bail!("port {port} in use: {e}（Palm 已在菜单栏运行）");
        }
        Err(e) => return Err(e.into()),
    };
    println!("✅ 就绪。等待手机接入…\n");

    // 手动 accept 循环：逐连接设置 TCP_NODELAY（§10 低延迟保障），
    // axum 0.8 的 Serve 未暴露该选项，故直接用 hyper。
    async fn accept_loop(router: axum::Router, listener: tokio::net::TcpListener) {
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((stream, addr)) = accepted else { continue };
                    let _ = stream.set_nodelay(true);
                    let tower_svc = router.clone();
                    tokio::spawn(async move {
                        use hyper::service::service_fn;
                        use hyper_util::rt::TokioIo;
                        use tower::Service as _;
                        let hyper_service = service_fn(
                            move |request: hyper::Request<hyper::body::Incoming>| {
                                let mut req = request.map(axum::body::Body::new);
                                req.extensions_mut()
                                    .insert(axum::extract::ConnectInfo(addr));
                                tower_svc.clone().call(req)
                            },
                        );
                        if let Err(e) = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), hyper_service)
                            .with_upgrades()
                            .await
                        {
                            tracing::debug!(%addr, "http connection ended: {e}");
                        }
                    });
                }
            }
        }
    }

    // 全局退出监听：SIGTERM/SIGINT 直接终止进程。
    // GUI 模式下主线程在 NSApp.run 里，若只 break accept 循环会留下"僵尸 UI"
    // （端口已关但进程存活），必须硬退出。
    tokio::spawn(async {
        shutdown_signal().await;
        std::process::exit(0);
    });

    #[cfg(target_os = "macos")]
    if gui {
        // 配对/连接全部在应用内完成：菜单栏 → 显示配对窗口
        tokio::spawn(accept_loop(router, listener));
        return ui::run(hub, port); // 阻塞主线程跑 NSApp 事件循环
    }

    accept_loop(router, listener).await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        let mut sig =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        sig.recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        other => match other {
            _ => "unknown",
        },
    }
}
