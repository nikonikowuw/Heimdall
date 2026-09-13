//! GB/T 28181-2016 SIP UAS (User Agent Server / 注册服务器) 实现
//!
//! 核心能力：
//! 1. 5060 端口 UDP/TCP 双栈异步信令服务；
//! 2. RFC 3261 / RFC 2617 Digest MD5 注册鉴权状态机 (防重放、Nonce 300s 窗口、IP 暴力防刷)；
//! 3. Keepalive 心跳解析与设备在线状态追踪；
//! 4. Catalog 目录扫描与通道批量解析；
//! 5. 按需点播 (INVITE / 200 OK / ACK) 与闲时停流 (BYE) 呼叫控制。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use md5::{Digest, Md5};
use parking_lot::RwLock;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use types::SysGb28181Config;

use crate::error::MediaError;

/// SIP 消息结构
#[derive(Debug, Clone)]
pub struct SipMessage {
    pub is_request: bool,
    pub method: String,
    pub uri: String,
    pub status_code: u16,
    pub reason: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl SipMessage {
    pub fn parse(raw: &str) -> Option<Self> {
        let mut lines = raw.split("\r\n");
        let first_line = lines.next()?.trim();
        if first_line.is_empty() {
            return None;
        }

        let is_request;
        let mut method = String::new();
        let mut uri = String::new();
        let mut status_code = 0;
        let mut reason = String::new();

        if first_line.starts_with("SIP/2.0") {
            is_request = false;
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            status_code = parts[1].parse().ok()?;
            reason = parts[2..].join(" ");
        } else {
            is_request = true;
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            method = parts[0].to_uppercase();
            uri = parts[1].to_string();
        }

        let mut headers = HashMap::new();
        let mut reading_body = false;
        let mut body_lines = Vec::new();

        for line in lines {
            if reading_body {
                body_lines.push(line);
            } else if line.is_empty() {
                reading_body = true;
            } else if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_lowercase(), v.trim().to_string());
            }
        }

        let body = body_lines.join("\r\n");

        Some(Self {
            is_request,
            method,
            uri,
            status_code,
            reason,
            headers,
            body,
        })
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_lowercase()).map(String::as_str)
    }
}

/// 已注册的设备会话信息
#[derive(Debug, Clone)]
pub struct RegisteredDeviceSession {
    pub device_id: String,
    pub name: String,
    pub remote_addr: SocketAddr,
    pub transport: String,
    pub last_keepalive_mono_ms: u64,
    pub cseq: u32,
}

/// SIP 服务器向系统分发的业务事件
#[derive(Debug, Clone)]
pub enum Gb28181Event {
    DeviceRegistered {
        device_id: String,
        name: String,
        ip_addr: String,
        sip_port: u16,
        transport: String,
    },
    DeviceKeepalive {
        device_id: String,
    },
    ChannelsDiscovered {
        device_id: String,
        channels: Vec<types::Gb28181ChannelDto>,
    },
}

/// SIP 呼叫会话 (Dialog)
#[derive(Debug, Clone)]
pub struct SipDialog {
    pub call_id: String,
    pub channel_id: String,
    pub device_id: String,
    pub remote_addr: SocketAddr,
    pub from_tag: String,
    pub to_tag: String,
    pub cseq: u32,
}

/// Nonce 认证记录
#[derive(Debug, Clone)]
struct NonceRecord {
    created_mono_ms: u64,
}

/// Gb28181 SIP 服务端
pub struct Gb28181SipServer {
    config: RwLock<SysGb28181Config>,
    server_ip: RwLock<String>,
    devices: Arc<RwLock<HashMap<String, RegisteredDeviceSession>>>,
    dialogs: Arc<RwLock<HashMap<String, SipDialog>>>,
    nonces: Arc<RwLock<HashMap<String, NonceRecord>>>,
    ip_failures: Arc<RwLock<HashMap<String, (u32, u64)>>>, // IP -> (失败次数, 封禁时间)
    event_tx: mpsc::Sender<Gb28181Event>,
    running: Arc<AtomicBool>,
    invite_waiters: Arc<RwLock<HashMap<String, oneshot::Sender<SipMessage>>>>,
    udp_socket: Arc<RwLock<Option<Arc<UdpSocket>>>>,
}

impl std::fmt::Debug for Gb28181SipServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181SipServer")
            .field("running", &self.running.load(Ordering::SeqCst))
            .finish()
    }
}

impl Gb28181SipServer {
    pub fn new(config: SysGb28181Config, event_tx: mpsc::Sender<Gb28181Event>) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            server_ip: RwLock::new("127.0.0.1".to_string()),
            devices: Arc::new(RwLock::new(HashMap::new())),
            dialogs: Arc::new(RwLock::new(HashMap::new())),
            nonces: Arc::new(RwLock::new(HashMap::new())),
            ip_failures: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            running: Arc::new(AtomicBool::new(false)),
            invite_waiters: Arc::new(RwLock::new(HashMap::new())),
            udp_socket: Arc::new(RwLock::new(None)),
        })
    }

    /// 更新服务器广播 IP（供 SDP c= 声明使用）
    pub fn set_server_ip(&self, ip: String) {
        *self.server_ip.write() = ip;
    }

    /// 更新运行配置
    pub fn update_config(&self, new_cfg: SysGb28181Config) {
        *self.config.write() = new_cfg;
    }

    /// 获取当前配置
    pub fn config(&self) -> SysGb28181Config {
        self.config.read().clone()
    }

    /// 获取已注册设备总数及在线数
    pub fn device_counts(&self) -> (usize, usize) {
        let devs = self.devices.read();
        let total = devs.len();
        (total, total)
    }

    /// 启动 SIP 服务端后台监听器 (UDP 5060 与 TCP 5060)
    pub async fn start(
        self: &Arc<Self>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), MediaError> {
        let port = self.config.read().sip_port;
        let bind_addr = format!("0.0.0.0:{port}");

        let udp_socket = match UdpSocket::bind(&bind_addr).await {
            Ok(sock) => Arc::new(sock),
            Err(e) => {
                warn!(port, error = %e, "无法绑定 GB28181 UDP 端口 (若需要 <1024 端口请使用 setcap 或反向代理)");
                return Err(MediaError::Protocol(format!(
                    "绑定 GB28181 UDP {port} 失败: {e}"
                )));
            }
        };

        let tcp_listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => Some(l),
            Err(e) => {
                warn!(port, error = %e, "无法绑定 GB28181 TCP 端口");
                None
            }
        };

        self.running.store(true, Ordering::SeqCst);
        *self.udp_socket.write() = Some(udp_socket.clone());
        info!(port, "GB28181 原生 SIP 服务端已成功启动监听");

        let this = self.clone();
        let udp_sock_clone = udp_socket.clone();

        // 1. UDP 报文接收主循环
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                tokio::select! {
                    res = udp_sock_clone.recv_from(&mut buf) => {
                        match res {
                            Ok((len, peer)) => {
                                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                                    this.handle_sip_raw(text, peer, "udp", &udp_sock_clone).await;
                                }
                            }
                            Err(e) => {
                                debug!(error = %e, "UDP recv_from 异常");
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
            this.running.store(false, Ordering::SeqCst);
        });

        // 2. TCP 报文接收循环（若绑定成功）
        if let Some(listener) = tcp_listener {
            let this_tcp = self.clone();
            let udp_for_reply = udp_socket.clone();
            tokio::spawn(async move {
                while let Ok((mut stream, peer)) = listener.accept().await {
                    let this = this_tcp.clone();
                    let udp_reply = udp_for_reply.clone();
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt;
                        let mut buf = vec![0u8; 65535];
                        if let Ok(n) = stream.read(&mut buf).await {
                            if n > 0 {
                                if let Ok(text) = std::str::from_utf8(&buf[..n]) {
                                    this.handle_sip_raw(text, peer, "tcp", &udp_reply).await;
                                }
                            }
                        }
                    });
                }
            });
        }

        // 3. 心跳超时保活与僵尸设备清理循环
        let this_hb = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                if !this_hb.running.load(Ordering::SeqCst) {
                    break;
                }
                let hb_timeout_ms = (this_hb.config.read().heartbeat_timeout_sec as u64) * 1000;
                let now = current_mono_ms();
                let mut devs = this_hb.devices.write();
                devs.retain(|id, session| {
                    if now.saturating_sub(session.last_keepalive_mono_ms) > hb_timeout_ms {
                        info!(device_id = %id, "设备 GB28181 心跳超时，标记为离线");
                        false
                    } else {
                        true
                    }
                });
            }
        });

        Ok(())
    }

    /// 处理接收到的原始 SIP 报文字符串
    async fn handle_sip_raw(
        &self,
        raw_text: &str,
        peer: SocketAddr,
        transport: &str,
        socket: &Arc<UdpSocket>,
    ) {
        let Some(msg) = SipMessage::parse(raw_text) else {
            return;
        };

        if msg.is_request {
            match msg.method.as_str() {
                "REGISTER" => {
                    self.handle_register(&msg, peer, transport, socket).await;
                }
                "MESSAGE" => {
                    self.handle_message(&msg, peer, socket).await;
                }
                "BYE" => {
                    self.handle_bye(&msg, peer, socket).await;
                }
                _ => {
                    // 其他请求默认回 200 OK
                    let resp = self.build_response(&msg, 200, "OK", None, None);
                    let _ = socket.send_to(resp.as_bytes(), peer).await;
                }
            }
        } else {
            // SIP 响应 (处理 INVITE 200 OK 等)
            if let Some(cseq) = msg.get_header("cseq") {
                if cseq.contains("INVITE") {
                    let call_id = msg.get_header("call-id").unwrap_or_default().to_string();
                    let mut waiters = self.invite_waiters.write();
                    if let Some(tx) = waiters.remove(&call_id) {
                        let _ = tx.send(msg.clone());
                    }
                }
            }
        }
    }

    /// 处理设备注册 (REGISTER)
    async fn handle_register(
        &self,
        msg: &SipMessage,
        peer: SocketAddr,
        transport: &str,
        socket: &Arc<UdpSocket>,
    ) {
        let cfg = self.config.read().clone();
        let ip_key = peer.ip().to_string();

        // 1. IP 防爆破校验
        let now = current_mono_ms();
        {
            let failures = self.ip_failures.read();
            if let Some((count, lock_until)) = failures.get(&ip_key) {
                if *count >= 5 && now < *lock_until {
                    warn!(ip = %ip_key, "拒绝处理被封禁 IP 的 REGISTER 请求");
                    return;
                }
            }
        }

        // 提取 device_id
        let from = msg.get_header("from").unwrap_or_default();
        let device_id = extract_sip_id(from).unwrap_or_default();
        if device_id.is_empty() {
            let resp = self.build_response(msg, 400, "Bad Request", None, None);
            let _ = socket.send_to(resp.as_bytes(), peer).await;
            return;
        }

        // 2. 鉴权校验
        let auth_header = msg.get_header("authorization");
        match auth_header {
            None => {
                // 第一步：回送 401 Unauthorized 与 Digest Challenge
                let nonce = generate_nonce();
                self.nonces.write().insert(
                    nonce.clone(),
                    NonceRecord {
                        created_mono_ms: now,
                    },
                );
                let auth_val = format!(
                    "Digest realm=\"{}\", nonce=\"{}\", algorithm=MD5",
                    cfg.sip_domain, nonce
                );
                let resp = self.build_response(
                    msg,
                    401,
                    "Unauthorized",
                    Some(("WWW-Authenticate", &auth_val)),
                    None,
                );
                let _ = socket.send_to(resp.as_bytes(), peer).await;
            }
            Some(auth_val) => {
                // 第二步：校验 MD5
                let auth_params = parse_auth_params(auth_val);
                let nonce = auth_params.get("nonce").cloned().unwrap_or_default();

                let is_nonce_valid = {
                    let mut nonces = self.nonces.write();
                    if let Some(record) = nonces.remove(&nonce) {
                        // 300 秒有效期
                        now.saturating_sub(record.created_mono_ms) <= 300_000
                    } else {
                        false
                    }
                };

                let username = auth_params.get("username").cloned().unwrap_or_default();
                let realm = auth_params.get("realm").cloned().unwrap_or_default();
                let response = auth_params.get("response").cloned().unwrap_or_default();
                let uri = auth_params.get("uri").cloned().unwrap_or_default();

                let expected = compute_digest_response(
                    &username,
                    &realm,
                    &cfg.sip_password,
                    &nonce,
                    "REGISTER",
                    &uri,
                );

                if is_nonce_valid && response == expected {
                    // 认证成功：注册设备
                    self.devices.write().insert(
                        device_id.clone(),
                        RegisteredDeviceSession {
                            device_id: device_id.clone(),
                            name: format!("GB-{device_id}"),
                            remote_addr: peer,
                            transport: transport.to_string(),
                            last_keepalive_mono_ms: now,
                            cseq: 1,
                        },
                    );

                    let _ = self
                        .event_tx
                        .send(Gb28181Event::DeviceRegistered {
                            device_id: device_id.clone(),
                            name: format!("GB-{device_id}"),
                            ip_addr: peer.ip().to_string(),
                            sip_port: peer.port(),
                            transport: transport.to_string(),
                        })
                        .await;

                    info!(device_id = %device_id, peer = %peer, "GB28181 设备注册鉴权成功上线");

                    let resp = self.build_response(msg, 200, "OK", None, None);
                    let _ = socket.send_to(resp.as_bytes(), peer).await;

                    // 若启用了自动目录同步，延迟 500ms 后触发 Catalog Query
                    if cfg.auto_catalog_sync {
                        let this = self.devices.clone();
                        let dev_id_clone = device_id.clone();
                        let socket_clone = socket.clone();
                        let cfg_clone = cfg.clone();
                        let server_ip_clone = self.server_ip.read().clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let peer_opt = this.read().get(&dev_id_clone).map(|d| d.remote_addr);
                            if let Some(target_peer) = peer_opt {
                                send_catalog_query_packet(
                                    &socket_clone,
                                    target_peer,
                                    &cfg_clone,
                                    &server_ip_clone,
                                    &dev_id_clone,
                                )
                                .await;
                            }
                        });
                    }
                } else {
                    // 认证失败，累加计数
                    {
                        let mut failures = self.ip_failures.write();
                        let entry = failures.entry(ip_key.clone()).or_insert((0, 0));
                        entry.0 += 1;
                        if entry.0 >= 5 {
                            entry.1 = now + 300_000; // 锁定 300 秒
                            warn!(ip = %ip_key, "连续 5 次 SIP 鉴权失败，锁定 300 秒");
                        }
                    }
                    let resp = self.build_response(msg, 403, "Forbidden", None, None);
                    let _ = socket.send_to(resp.as_bytes(), peer).await;
                }
            }
        }
    }

    /// 处理 MESSAGE 消息 (Keepalive / Catalog Response)
    async fn handle_message(&self, msg: &SipMessage, peer: SocketAddr, socket: &Arc<UdpSocket>) {
        // 先响应 200 OK
        let resp = self.build_response(msg, 200, "OK", None, None);
        let _ = socket.send_to(resp.as_bytes(), peer).await;

        let body = &msg.body;

        // 1. 心跳包解析: <CmdType>Keepalive</CmdType>
        if body.contains("<CmdType>Keepalive</CmdType>") {
            if let Some(dev_id) = extract_xml_tag(body, "DeviceID") {
                {
                    let mut devs = self.devices.write();
                    if let Some(dev) = devs.get_mut(&dev_id) {
                        dev.last_keepalive_mono_ms = current_mono_ms();
                        dev.remote_addr = peer;
                        debug!(device_id = %dev_id, "收到 GB28181 心跳包");
                    }
                }
                let _ = self
                    .event_tx
                    .send(Gb28181Event::DeviceKeepalive { device_id: dev_id })
                    .await;
            }
        }
        // 2. 目录响应解析: <CmdType>Catalog</CmdType>
        else if body.contains("<CmdType>Catalog</CmdType>") {
            let parent_dev = extract_xml_tag(body, "DeviceID").unwrap_or_default();
            let channels = parse_catalog_items(body, &parent_dev);
            info!(
                device_id = %parent_dev,
                channels_count = channels.len(),
                "解析出 GB28181 设备目录通道"
            );
            let _ = self
                .event_tx
                .send(Gb28181Event::ChannelsDiscovered {
                    device_id: parent_dev,
                    channels,
                })
                .await;
        }
    }

    /// 处理 BYE 拆链请求
    async fn handle_bye(&self, msg: &SipMessage, peer: SocketAddr, socket: &Arc<UdpSocket>) {
        let call_id = msg.get_header("call-id").unwrap_or_default();
        self.dialogs.write().remove(call_id);
        let resp = self.build_response(msg, 200, "OK", None, None);
        let _ = socket.send_to(resp.as_bytes(), peer).await;
    }

    /// 向目标设备通道发起 INVITE 点播流
    pub async fn invite_stream(
        &self,
        device_id: &str,
        channel_id: &str,
        rtp_port: u16,
        is_tcp: bool,
    ) -> Result<String, MediaError> {
        let (target_peer, cseq) = {
            let devs = self.devices.read();
            let dev = devs.get(device_id).ok_or_else(|| {
                MediaError::Protocol(format!("国标设备 {device_id} 不在线或未注册"))
            })?;
            (dev.remote_addr, dev.cseq + 1)
        };

        let cfg = self.config.read().clone();
        let ssrc = generate_gb28181_ssrc(&cfg.sip_domain);
        let call_id = format!("{}_{}", current_mono_ms(), uuid::Uuid::now_v7().simple());
        let from_tag = format!("{:x}", rand_u32());
        let server_ip = self.server_ip.read().clone();

        let sdp = if is_tcp {
            format!(
                "v=0\r\n\
                 o={} 0 0 IN IP4 {}\r\n\
                 s=Play\r\n\
                 c=IN IP4 {}\r\n\
                 t=0 0\r\n\
                 m=video {} TCP/RTP/AVP 96\r\n\
                 a=recvonly\r\n\
                 a=rtpmap:96 PS/90000\r\n\
                 a=setup:passive\r\n\
                 a=connection:new\r\n\
                 y={}\r\n",
                cfg.sip_id, server_ip, server_ip, rtp_port, ssrc
            )
        } else {
            format!(
                "v=0\r\n\
                 o={} 0 0 IN IP4 {}\r\n\
                 s=Play\r\n\
                 c=IN IP4 {}\r\n\
                 t=0 0\r\n\
                 m=video {} RTP/AVP 96\r\n\
                 a=recvonly\r\n\
                 a=rtpmap:96 PS/90000\r\n\
                 y={}\r\n",
                cfg.sip_id, server_ip, server_ip, rtp_port, ssrc
            )
        };

        let invite_msg = format!(
            "INVITE sip:{}@{} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {}:{};branch=z9hG4bK{}\r\n\
             From: <sip:{}@{}>;tag={}\r\n\
             To: <sip:{}@{}>\r\n\
             Call-ID: {}\r\n\
             CSeq: {} INVITE\r\n\
             Contact: <sip:{}@{}:{}>\r\n\
             Content-Type: application/sdp\r\n\
             Max-Forwards: 70\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            channel_id,
            cfg.sip_domain,
            server_ip,
            cfg.sip_port,
            rand_u32(),
            cfg.sip_id,
            cfg.sip_domain,
            from_tag,
            channel_id,
            cfg.sip_domain,
            call_id,
            cseq,
            cfg.sip_id,
            server_ip,
            cfg.sip_port,
            sdp.len(),
            sdp
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.invite_waiters
            .write()
            .insert(call_id.clone(), reply_tx);

        // 发送 INVITE (优先使用当前监听套接字发送，否则绑定临时 UDP 套接字)
        let sock_opt = self.udp_socket.read().clone();
        if let Some(sock) = sock_opt.as_ref() {
            sock.send_to(invite_msg.as_bytes(), target_peer)
                .await
                .map_err(|e| MediaError::Protocol(format!("发送 SIP INVITE 失败: {e}")))?;
        } else {
            let sock = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| {
                MediaError::Protocol(format!("无法创建 SIP INVITE UDP 套接字: {e}"))
            })?;
            sock.send_to(invite_msg.as_bytes(), target_peer)
                .await
                .map_err(|e| MediaError::Protocol(format!("发送 SIP INVITE 失败: {e}")))?;
        }

        // 等待 200 OK 响应 (15 秒硬超时)
        let resp = match tokio::time::timeout(Duration::from_secs(15), reply_rx).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_)) => {
                return Err(MediaError::Protocol(
                    "SIP INVITE 响应通道异常中断".to_string(),
                ))
            }
            Err(_) => {
                self.invite_waiters.write().remove(&call_id);
                return Err(MediaError::Protocol(
                    "SIP INVITE 协商超时 (15s)".to_string(),
                ));
            }
        };

        if resp.status_code != 200 {
            return Err(MediaError::Protocol(format!(
                "SIP INVITE 被拒绝: {} {}",
                resp.status_code, resp.reason
            )));
        }

        let to_tag = resp
            .get_header("to")
            .and_then(|t| t.split("tag=").nth(1))
            .unwrap_or_default()
            .to_string();

        // 发送 ACK
        let ack_msg = format!(
            "ACK sip:{}@{} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {}:{};branch=z9hG4bK{}\r\n\
             From: <sip:{}@{}>;tag={}\r\n\
             To: <sip:{}@{}>;tag={}\r\n\
             Call-ID: {}\r\n\
             CSeq: {} ACK\r\n\
             Max-Forwards: 70\r\n\
             Content-Length: 0\r\n\
             \r\n",
            channel_id,
            cfg.sip_domain,
            server_ip,
            cfg.sip_port,
            rand_u32(),
            cfg.sip_id,
            cfg.sip_domain,
            from_tag,
            channel_id,
            cfg.sip_domain,
            to_tag,
            call_id,
            cseq
        );
        if let Some(sock) = sock_opt.as_ref() {
            let _ = sock.send_to(ack_msg.as_bytes(), target_peer).await;
        } else if let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await {
            let _ = sock.send_to(ack_msg.as_bytes(), target_peer).await;
        }

        self.dialogs.write().insert(
            call_id.clone(),
            SipDialog {
                call_id: call_id.clone(),
                channel_id: channel_id.to_string(),
                device_id: device_id.to_string(),
                remote_addr: target_peer,
                from_tag,
                to_tag,
                cseq: cseq + 1,
            },
        );

        info!(
            device_id,
            channel_id, rtp_port, "GB28181 点播成功完成握手推流"
        );
        Ok(call_id)
    }

    /// 主动向指定设备触发 Catalog 目录查询
    pub async fn sync_device_catalog(&self, device_id: &str) -> Result<(), MediaError> {
        let (peer, cfg, server_ip) = {
            let devs = self.devices.read();
            let dev = devs.get(device_id).ok_or_else(|| {
                MediaError::Protocol(format!("国标设备 {device_id} 不在线或未注册"))
            })?;
            (
                dev.remote_addr,
                self.config.read().clone(),
                self.server_ip.read().clone(),
            )
        };

        let sock_opt = self.udp_socket.read().clone();
        if let Some(sock) = sock_opt.as_ref() {
            send_catalog_query_packet(sock, peer, &cfg, &server_ip, device_id).await;
            Ok(())
        } else {
            let sock = UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| MediaError::Protocol(format!("无法创建 SIP UDP 套接字: {e}")))?;
            send_catalog_query_packet(&sock, peer, &cfg, &server_ip, device_id).await;
            Ok(())
        }
    }

    /// 注册测试设备会话（供测试使用）
    pub fn register_test_session(&self, session: RegisteredDeviceSession) {
        self.devices
            .write()
            .insert(session.device_id.clone(), session);
    }

    /// 获取设备的传输协议
    pub fn get_device_transport(&self, device_id: &str) -> Option<String> {
        self.devices
            .read()
            .get(device_id)
            .map(|d| d.transport.clone())
    }

    /// 停止流传输 (BYE)
    pub async fn stop_stream(&self, call_id: &str) {
        let dialog = self.dialogs.write().remove(call_id);
        if let Some(dlg) = dialog {
            let cfg = self.config.read().clone();
            let server_ip = self.server_ip.read().clone();
            let bye_msg = format!(
                "BYE sip:{}@{} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {}:{};branch=z9hG4bK{}\r\n\
                 From: <sip:{}@{}>;tag={}\r\n\
                 To: <sip:{}@{}>;tag={}\r\n\
                 Call-ID: {}\r\n\
                 CSeq: {} BYE\r\n\
                 Max-Forwards: 70\r\n\
                 Content-Length: 0\r\n\
                 \r\n",
                dlg.channel_id,
                cfg.sip_domain,
                server_ip,
                cfg.sip_port,
                rand_u32(),
                cfg.sip_id,
                cfg.sip_domain,
                dlg.from_tag,
                dlg.channel_id,
                cfg.sip_domain,
                dlg.to_tag,
                call_id,
                dlg.cseq
            );

            let sock_opt = self.udp_socket.read().clone();
            if let Some(sock) = sock_opt.as_ref() {
                let _ = sock.send_to(bye_msg.as_bytes(), dlg.remote_addr).await;
            } else if let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await {
                let _ = sock.send_to(bye_msg.as_bytes(), dlg.remote_addr).await;
            }
        }
    }

    /// 构造标准 SIP 响应
    fn build_response(
        &self,
        req: &SipMessage,
        status_code: u16,
        reason: &str,
        extra_header: Option<(&str, &str)>,
        body: Option<&str>,
    ) -> String {
        let via = req.get_header("via").unwrap_or_default();
        let from = req.get_header("from").unwrap_or_default();
        let to = req.get_header("to").unwrap_or_default();
        let call_id = req.get_header("call-id").unwrap_or_default();
        let cseq = req.get_header("cseq").unwrap_or_default();

        let to_with_tag = if !to.contains("tag=") {
            format!("{to};tag={:x}", rand_u32())
        } else {
            to.to_string()
        };

        let mut resp = format!(
            "SIP/2.0 {status_code} {reason}\r\n\
             Via: {via}\r\n\
             From: {from}\r\n\
             To: {to_with_tag}\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: {cseq}\r\n"
        );

        if let Some((k, v)) = extra_header {
            resp.push_str(&format!("{k}: {v}\r\n"));
        }

        let body_str = body.unwrap_or_default();
        resp.push_str(&format!("Content-Length: {}\r\n\r\n", body_str.len()));
        if !body_str.is_empty() {
            resp.push_str(body_str);
        }

        resp
    }
}

/// 发送 Catalog 查询报文
async fn send_catalog_query_packet(
    socket: &UdpSocket,
    peer: SocketAddr,
    cfg: &SysGb28181Config,
    server_ip: &str,
    target_id: &str,
) {
    let sn = rand_u32() % 100000;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"GB2312\"?>\r\n\
         <Query>\r\n\
           <CmdType>Catalog</CmdType>\r\n\
           <SN>{sn}</SN>\r\n\
           <DeviceID>{target_id}</DeviceID>\r\n\
         </Query>\r\n"
    );

    let msg = format!(
        "MESSAGE sip:{target_id}@{peer} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {}:{};branch=z9hG4bK{}\r\n\
         From: <sip:{}@{}>;tag={}\r\n\
         To: <sip:{}@{}>\r\n\
         Call-ID: {}_catalog\r\n\
         CSeq: 1 MESSAGE\r\n\
         Content-Type: Application/MANSCDP+xml\r\n\
         Max-Forwards: 70\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        server_ip,
        cfg.sip_port,
        rand_u32(),
        cfg.sip_id,
        cfg.sip_domain,
        rand_u32(),
        target_id,
        cfg.sip_domain,
        current_mono_ms(),
        xml.len(),
        xml
    );

    let _ = socket.send_to(msg.as_bytes(), peer).await;
}

/// 解析 Catalog XML 返回的通道列表
fn parse_catalog_items(xml: &str, default_parent: &str) -> Vec<types::Gb28181ChannelDto> {
    let mut channels = Vec::new();
    let now = chrono::Utc::now().timestamp_millis();

    // 简单高效的流式提取 <Item>...</Item>
    let mut rest = xml;
    while let Some(start_idx) = rest.find("<Item>") {
        let after_start = &rest[start_idx + 6..];
        let Some(end_idx) = after_start.find("</Item>") else {
            break;
        };
        let item_xml = &after_start[..end_idx];

        let ch_id = extract_xml_tag(item_xml, "DeviceID").unwrap_or_default();
        if !ch_id.is_empty() {
            let name = extract_xml_tag(item_xml, "Name").unwrap_or_else(|| ch_id.clone());
            let mfr = extract_xml_tag(item_xml, "Manufacturer").unwrap_or_default();
            let model = extract_xml_tag(item_xml, "Model").unwrap_or_default();
            let status = extract_xml_tag(item_xml, "Status").unwrap_or_else(|| "ON".to_string());
            let parent =
                extract_xml_tag(item_xml, "ParentID").unwrap_or_else(|| default_parent.to_string());

            channels.push(types::Gb28181ChannelDto {
                device_id: default_parent.to_string(),
                channel_id: ch_id,
                name,
                manufacturer: mfr,
                model,
                status,
                parent_id: parent,
                sub_stream_supported: true,
                last_seen_ms: now,
                is_imported: false,
                camera_id: None,
            });
        }

        rest = &after_start[end_idx + 7..];
    }

    channels
}

/// 简单标签提取器
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{tag}>");
    let close_tag = format!("</{tag}>");

    let start = xml.find(&open_tag)? + open_tag.len();
    let end = start + xml[start..].find(&close_tag)?;
    Some(xml[start..end].trim().to_string())
}

/// 提取 URI 中的国标 ID
fn extract_sip_id(uri_str: &str) -> Option<String> {
    let sip_pos = uri_str.find("sip:")?;
    let rest = &uri_str[sip_pos + 4..];
    let end = rest.find(['@', '>', ';', ':'])?;
    Some(rest[..end].trim().to_string())
}

/// 解析 Authorization 头中的键值对
fn parse_auth_params(header_val: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let trimmed = header_val.trim_start_matches("Digest ").trim();
    for part in trimmed.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            let key = k.trim().to_lowercase();
            let val = v.trim().trim_matches('"').to_string();
            map.insert(key, val);
        }
    }
    map
}

/// 计算 Digest MD5 Response
fn compute_digest_response(
    username: &str,
    realm: &str,
    password: &str,
    nonce: &str,
    method: &str,
    uri: &str,
) -> String {
    // HA1 = MD5(username:realm:password)
    let ha1 = format!(
        "{:x}",
        Md5::digest(format!("{username}:{realm}:{password}"))
    );
    // HA2 = MD5(method:uri)
    let ha2 = format!("{:x}", Md5::digest(format!("{method}:{uri}")));
    // Response = MD5(HA1:nonce:HA2)
    format!("{:x}", Md5::digest(format!("{ha1}:{nonce}:{ha2}")))
}

fn generate_nonce() -> String {
    format!("{:x}{:x}", rand_u32(), rand_u32())
}

fn generate_gb28181_ssrc(domain: &str) -> String {
    let domain_suffix = if domain.len() >= 4 {
        &domain[domain.len() - 4..]
    } else {
        "0000"
    };
    format!("0{domain_suffix}{:05}", rand_u32() % 100000)
}

fn rand_u32() -> u32 {
    let now = current_mono_ms() as u32;
    let pid = std::process::id();
    now.wrapping_mul(1103515245).wrapping_add(pid)
}

fn current_mono_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_message_parse() {
        let raw = "REGISTER sip:34020000002000000001@3402000000 SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.50:5060;rport;branch=z9hG4bK12345\r\n\
                   From: <sip:34020000001180000001@3402000000>;tag=67890\r\n\
                   To: <sip:34020000002000000001@3402000000>\r\n\
                   Call-ID: 99999@192.168.1.50\r\n\
                   CSeq: 1 REGISTER\r\n\
                   Contact: <sip:34020000001180000001@192.168.1.50:5060>\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse sip");
        assert!(msg.is_request);
        assert_eq!(msg.method, "REGISTER");
        assert_eq!(msg.get_header("cseq"), Some("1 REGISTER"));
        assert_eq!(
            extract_sip_id(msg.get_header("from").unwrap()),
            Some("34020000001180000001".to_string())
        );
    }

    #[test]
    fn test_digest_md5_computation() {
        let resp = compute_digest_response(
            "34020000001180000001",
            "3402000000",
            "admin123",
            "abc123nonce",
            "REGISTER",
            "sip:34020000002000000001@3402000000",
        );
        assert_eq!(resp.len(), 32);
    }

    #[test]
    fn test_catalog_xml_parse() {
        let xml = "<?xml version=\"1.0\" encoding=\"GB2312\"?>\r\n\
                   <Response>\r\n\
                     <CmdType>Catalog</CmdType>\r\n\
                     <SN>100</SN>\r\n\
                     <DeviceID>34020000001180000001</DeviceID>\r\n\
                     <SumNum>1</SumNum>\r\n\
                     <DeviceList Num=\"1\">\r\n\
                       <Item>\r\n\
                         <DeviceID>34020000001310000001</DeviceID>\r\n\
                         <Name>Front Entrance</Name>\r\n\
                         <Manufacturer>Hikvision</Manufacturer>\r\n\
                         <Model>DS-2CD2047G2</Model>\r\n\
                         <Status>ON</Status>\r\n\
                       </Item>\r\n\
                     </DeviceList>\r\n\
                   </Response>";

        let channels = parse_catalog_items(xml, "34020000001180000001");
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].channel_id, "34020000001310000001");
        assert_eq!(channels[0].name, "Front Entrance");
        assert_eq!(channels[0].manufacturer, "Hikvision");
    }
}
