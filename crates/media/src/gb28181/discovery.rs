//! 局域网摄像头主动嗅探引擎 (Active LAN Discovery)
//!
//! 1. 基于 ONVIF WS-Discovery 组播协议向 `239.255.255.250:3702` 发送 SOAP Probe 探测包；
//! 2. 3 秒超时窗口内平滑收集回复报文，解析摄像机 IP、XAddrs、厂商及硬件型号；
//! 3. 对目标 IP 并发进行 554 RTSP 端口探活，输出即开即用的 `DiscoveredDevice` 列表。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tracing::info;
use types::DiscoveredDevice;

use crate::error::MediaError;

/// ONVIF WS-Discovery 组播组地址
const WS_DISCOVERY_MULTICAST_ADDR: &str = "239.255.255.250:3702";

/// 触发局域网摄像头主动扫描
pub async fn scan_lan_cameras(timeout: Duration) -> Result<Vec<DiscoveredDevice>, MediaError> {
    let bind_addr = "0.0.0.0:0";
    let socket = UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| MediaError::Protocol(format!("绑定组播扫描套接字失败: {e}")))?;

    // 设置广播权限
    let _ = socket.set_broadcast(true);

    let msg_id = format!("uuid:{}", uuid::Uuid::now_v7());
    let probe_xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
         <Envelope xmlns=\"http://www.w3.org/2003/05/soap-envelope\" \
                   xmlns:dn=\"http://www.onvif.org/ver10/network/wsdl\">\r\n\
           <Header>\r\n\
             <wsa:MessageID xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\">{msg_id}</wsa:MessageID>\r\n\
             <wsa:To xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\">urn:schemas-xmlsoap-org:ws:2005/04/discovery</wsa:To>\r\n\
             <wsa:Action xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\">http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</wsa:Action>\r\n\
           </Header>\r\n\
           <Body>\r\n\
             <dn:Probe>\r\n\
               <dn:Types>dn:NetworkVideoTransmitter</dn:Types>\r\n\
             </dn:Probe>\r\n\
           </Body>\r\n\
         </Envelope>"
    );

    let target: SocketAddr = WS_DISCOVERY_MULTICAST_ADDR
        .parse()
        .map_err(|e| MediaError::Protocol(format!("解析组播地址失败: {e}")))?;

    let _ = socket.send_to(probe_xml.as_bytes(), target).await;

    let mut discovered: HashMap<String, DiscoveredDevice> = HashMap::new();
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        tokio::select! {
            res = tokio::time::timeout(remaining, socket.recv_from(&mut buf)) => {
                match res {
                    Ok(Ok((len, peer))) => {
                        if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                            if text.contains("ProbeMatches") {
                                let dev = parse_probe_match(text, peer);
                                discovered.insert(dev.ip.clone(), dev);
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
    }

    // 对嗅探到的在线设备并发执行 RTSP 554 端口快速探活
    let mut probe_tasks = Vec::new();
    for (_, mut dev) in discovered {
        probe_tasks.push(async move {
            let addr = format!("{}:554", dev.ip);
            let is_open = matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    tokio::net::TcpStream::connect(&addr),
                )
                .await,
                Ok(Ok(_))
            );

            if is_open {
                dev.rtsp_url = Some(format!("rtsp://{}:554/live/ch0", dev.ip));
            } else {
                dev.rtsp_url = None;
            }
            dev
        });
    }

    let devices: Vec<DiscoveredDevice> = futures::future::join_all(probe_tasks).await;
    info!(
        count = devices.len(),
        "ONVIF 局域网组播嗅探与 RTSP 探活完成"
    );
    Ok(devices)
}

/// 从 SOAP ProbeMatch 报文中提取信息
fn parse_probe_match(xml: &str, peer: SocketAddr) -> DiscoveredDevice {
    let xaddrs = extract_tag_content(xml, "XAddrs");
    let scopes = extract_tag_content(xml, "Scopes").unwrap_or_default();

    let mut name = format!("IPC-{}", peer.ip());
    let mut mfr = "Standard IPC".to_string();
    let mut model = "ONVIF Device".to_string();

    for part in scopes.split_whitespace() {
        if let Some(val) = part.strip_prefix("onvif://www.onvif.org/name/") {
            name = percent_encoding::percent_decode_str(val)
                .decode_utf8_lossy()
                .to_string();
        } else if let Some(val) = part.strip_prefix("onvif://www.onvif.org/hardware/") {
            model = percent_encoding::percent_decode_str(val)
                .decode_utf8_lossy()
                .to_string();
        } else if let Some(val) = part.strip_prefix("onvif://www.onvif.org/manufacturer/") {
            mfr = percent_encoding::percent_decode_str(val)
                .decode_utf8_lossy()
                .to_string();
        }
    }

    let ip = peer.ip().to_string();
    let port = peer.port();

    DiscoveredDevice {
        ip,
        port,
        name,
        manufacturer: mfr,
        model,
        protocol: "onvif".to_string(),
        xaddrs,
        rtsp_url: None, // 由后续探活步骤填充
    }
}

fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}>");

    let tag_start = xml.find(&open_pat)?;
    let after_open = &xml[tag_start..];
    let content_start = after_open.find('>')? + 1;
    let content_end = after_open.find(&close_pat)?;

    Some(after_open[content_start..content_end].trim().to_string())
}
