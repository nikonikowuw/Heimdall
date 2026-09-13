//! GB/T 28181 媒体流接入引擎 (Gb28181Ingestor)
//!
//! 负责单路 GB28181 摄像头通道的媒体数据面拉流：
//! 1. 动态租借 RTP 接收端口对 (PortPool)；
//! 2. 驱动 SIP UAS 服务端向远端设备下发 INVITE 点播；
//! 3. 异步监听 UDP/TCP RTP 套接字，通过 JitterBuffer 乱序重排与 PS 解复用器提取 Annex-B NALU；
//! 4. 组装 `EncodedPacket` 并直接注入 StreamHub 的独立 Mailbox 分发总线 (`PacketDispatcher`)；
//! 5. 客户端全断开或停机时，发送 BYE 拆链并安全归还端口租约。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, UdpSocket};
use tracing::{debug, error, info, warn};

use crate::dispatcher::PacketDispatcher;
use crate::gb28181::port_pool::PortPool;
use crate::gb28181::ps::PsDemuxer;
use crate::gb28181::rtp::{JitterBuffer, RtpPacket};
use crate::gb28181::sip::Gb28181SipServer;

/// 解析 `gb28181://{deviceId}/{channelId}` 格式的 URL
pub fn parse_gb28181_url(url_str: &str) -> Option<(String, String)> {
    let trimmed = url_str.trim();
    let without_schema = trimmed.strip_prefix("gb28181://")?;
    let (dev, ch) = without_schema.split_once('/')?;
    let dev = dev.trim();
    let ch = ch.trim();
    if dev.is_empty() || ch.is_empty() {
        return None;
    }
    Some((dev.to_string(), ch.to_string()))
}

/// GB28181 媒体数据流接收器
#[derive(Debug)]
pub struct Gb28181Ingestor {
    device_id: String,
    channel_id: String,
    sip_server: Arc<Gb28181SipServer>,
    port_pool: PortPool,
    dispatcher: Arc<PacketDispatcher>,
}

impl Gb28181Ingestor {
    pub fn new(
        device_id: impl Into<String>,
        channel_id: impl Into<String>,
        sip_server: Arc<Gb28181SipServer>,
        port_pool: PortPool,
        dispatcher: Arc<PacketDispatcher>,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            channel_id: channel_id.into(),
            sip_server,
            port_pool,
            dispatcher,
        }
    }

    /// 运行国标拉流主循环
    pub async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        let dev_id = &self.device_id;
        let ch_id = &self.channel_id;

        info!(device_id = %dev_id, channel_id = %ch_id, "启动 GB28181 数据面接入任务");

        // 1. 申请动态媒体端口
        let port_lease = match self.port_pool.allocate_pair() {
            Ok(lease) => lease,
            Err(e) => {
                error!(error = %e, "分配 GB28181 媒体端口失败");
                return;
            }
        };

        let rtp_port = port_lease.rtp_port;
        let rtp_bind_addr = format!("0.0.0.0:{rtp_port}");

        // 判断设备首选传输方式 (默认推荐 TCP 被动模式以防公网/弱网 UDP 丢包乱序)
        let transport = self
            .sip_server
            .get_device_transport(dev_id)
            .unwrap_or_else(|| "tcp".to_string());
        let use_tcp = transport.eq_ignore_ascii_case("tcp");

        let mut packets_received: u64 = 0;
        let mut ps_demuxer = PsDemuxer::new();

        if use_tcp {
            // TCP 被动模式 (RFC 4571)
            let tcp_listener = match TcpListener::bind(&rtp_bind_addr).await {
                Ok(l) => l,
                Err(e) => {
                    error!(port = rtp_port, error = %e, "绑定 TCP RTP 接收端口失败");
                    return;
                }
            };

            let call_id = match self
                .sip_server
                .invite_stream(dev_id, ch_id, rtp_port, true)
                .await
            {
                Ok(cid) => cid,
                Err(e) => {
                    error!(device_id = %dev_id, channel_id = %ch_id, error = %e, "SIP INVITE (TCP) 握手失败");
                    return;
                }
            };

            info!(
                device_id = %dev_id,
                channel_id = %ch_id,
                rtp_port,
                call_id = %call_id,
                "GB28181 TCP 点播流建立完成，等待设备主动发起 TCP 媒体连接"
            );

            // 等待 IPC 建立 TCP 连接 (15 秒超时)
            let mut tcp_stream = tokio::select! {
                accept_res = tcp_listener.accept() => {
                    match accept_res {
                        Ok((stream, peer)) => {
                            info!(peer = %peer, "设备 TCP 媒体流连接已就绪");
                            stream
                        }
                        Err(e) => {
                            error!(error = %e, "接受设备 TCP 媒体连接失败");
                            self.sip_server.stop_stream(&call_id).await;
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                    error!("等待设备 TCP 媒体连接超时 (15s)");
                    self.sip_server.stop_stream(&call_id).await;
                    return;
                }
                _ = cancel_rx.changed() => {
                    self.sip_server.stop_stream(&call_id).await;
                    return;
                }
            };

            let mut len_buf = [0u8; 2];
            loop {
                if cancel_signal.load(Ordering::SeqCst) {
                    break;
                }

                tokio::select! {
                    res = tcp_stream.read_exact(&mut len_buf) => {
                        match res {
                            Ok(_) => {
                                let packet_len = u16::from_be_bytes(len_buf) as usize;
                                if !(12..=65535).contains(&packet_len) {
                                    warn!(packet_len, "RFC 4571 报文长度异常，忽略本切片");
                                    continue;
                                }
                                let mut packet_buf = vec![0u8; packet_len];
                                if tcp_stream.read_exact(&mut packet_buf).await.is_err() {
                                    break;
                                }
                                if let Ok(rtp_pkt) = RtpPacket::parse(&packet_buf) {
                                    let es_packets = ps_demuxer.demux(&rtp_pkt.payload);
                                    for packet in es_packets {
                                        packets_received += 1;
                                        self.dispatcher.publish(Arc::new(packet));
                                    }
                                }
                            }
                            Err(e) => {
                                debug!(error = %e, "TCP RTP 媒体流断开");
                                break;
                            }
                        }
                    }
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            break;
                        }
                    }
                }
            }

            if let Some(final_packet) = ps_demuxer.flush() {
                self.dispatcher.publish(Arc::new(final_packet));
            }

            info!(
                device_id = %dev_id,
                channel_id = %ch_id,
                packets_received,
                "GB28181 TCP 媒体流会话退出，发送 BYE 拆链"
            );
            self.sip_server.stop_stream(&call_id).await;
            drop(port_lease);
        } else {
            // UDP 兼容模式
            let rtp_socket = match UdpSocket::bind(&rtp_bind_addr).await {
                Ok(sock) => sock,
                Err(e) => {
                    error!(port = rtp_port, error = %e, "绑定 UDP RTP 接收端口失败");
                    return;
                }
            };

            let call_id = match self
                .sip_server
                .invite_stream(dev_id, ch_id, rtp_port, false)
                .await
            {
                Ok(cid) => cid,
                Err(e) => {
                    error!(device_id = %dev_id, channel_id = %ch_id, error = %e, "SIP INVITE (UDP) 握手失败");
                    return;
                }
            };

            info!(
                device_id = %dev_id,
                channel_id = %ch_id,
                rtp_port,
                call_id = %call_id,
                "GB28181 UDP 点播流建立完成，开始接收 RTP/PS 媒体数据"
            );

            let mut jitter_buffer = JitterBuffer::new(64);
            let mut recv_buf = vec![0u8; 65535];

            loop {
                if cancel_signal.load(Ordering::SeqCst) {
                    break;
                }

                tokio::select! {
                    res = rtp_socket.recv_from(&mut recv_buf) => {
                        match res {
                            Ok((len, _peer)) => {
                                if len < 12 {
                                    continue;
                                }
                                if let Ok(rtp_pkt) = RtpPacket::parse(&recv_buf[..len]) {
                                    let in_order_pkts = jitter_buffer.push(rtp_pkt);
                                    for ordered in in_order_pkts {
                                        if ordered.discontinuity {
                                            ps_demuxer.reset_on_discontinuity();
                                        }
                                        let es_packets = ps_demuxer.demux(&ordered.payload);
                                        for packet in es_packets {
                                            packets_received += 1;
                                            self.dispatcher.publish(Arc::new(packet));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!(error = %e, "UDP RTP 接收中断");
                                break;
                            }
                        }
                    }
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            break;
                        }
                    }
                }
            }

            if let Some(final_packet) = ps_demuxer.flush() {
                self.dispatcher.publish(Arc::new(final_packet));
            }

            info!(
                device_id = %dev_id,
                channel_id = %ch_id,
                packets_received,
                "GB28181 UDP 媒体流会话退出，发送 BYE 拆链"
            );
            self.sip_server.stop_stream(&call_id).await;
            drop(port_lease);
        }
    }
}

#[async_trait::async_trait]
impl crate::media_ingestor::MediaIngestor for Gb28181Ingestor {
    async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        Self::run_loop(self, cancel_signal, cancel_rx).await;
    }
}
