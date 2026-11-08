//! mDNS 服务发现（§3）：广播 _palm._tcp，手机端兜底发现。
//! 主路径是二维码直连 URL，mDNS 只是辅助。

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub const SERVICE_TYPE: &str = "_palm._tcp.local.";

/// 取局域网主 IP（UDP connect 探测，不发包）。
pub fn primary_lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() {
        None
    } else {
        Some(ip.to_string())
    }
}

pub fn advertise(name: &str, port: u16) -> anyhow::Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new()?;
    let props = [
        ("name".to_string(), name.to_string()),
        ("ver".to_string(), env!("CARGO_PKG_VERSION").to_string()),
    ];
    let host = format!("palm-{}.local.", primary_lan_ip().unwrap_or_default().replace('.', "-"));
    let instance = format!("palm-{port}");
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &host,
        "",
        port,
        Some(props.into_iter().collect()),
    )?
    .enable_addr_auto();
    mdns.register(service)?;
    tracing::info!("mDNS advertised: {SERVICE_TYPE} instance={instance}");
    Ok(mdns)
}
