//! 动态媒体端口池 (Port Pool)
//!
//! 管理 GB28181 接收 RTP/RTCP 媒体流的 UDP/TCP 端口：
//! 1. 偶数端口接收 RTP 数据流，奇数端口 (RTP + 1) 预留接收 RTCP；
//! 2. 具备 RAII `PortLease` 租约，点播结束、客户端断开或异常时自动归还端口，物理杜绝端口泄漏；
//! 3. 线程安全并发分配与回收。

use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use crate::error::MediaError;

/// RAII 端口租约
#[derive(Debug)]
pub struct PortLease {
    pub rtp_port: u16,
    pub rtcp_port: u16,
    pool: Arc<PortPoolInner>,
}

impl Drop for PortLease {
    fn drop(&mut self) {
        self.pool.release(self.rtp_port);
    }
}

#[derive(Debug)]
struct PortPoolInner {
    start: u16,
    end: u16,
    used: Mutex<HashSet<u16>>,
    next_hint: AtomicU16,
}

impl PortPoolInner {
    fn release(&self, rtp_port: u16) {
        let mut used = self.used.lock();
        used.remove(&rtp_port);
        used.remove(&(rtp_port + 1));
    }
}

/// 媒体端口池管理器
#[derive(Debug, Clone)]
pub struct PortPool {
    inner: Arc<PortPoolInner>,
}

impl PortPool {
    pub fn new(range_start: u16, range_end: u16) -> Self {
        // 确保 start 为偶数
        let start = if !range_start.is_multiple_of(2) {
            range_start + 1
        } else {
            range_start
        };
        let end = range_end.max(start + 2);

        Self {
            inner: Arc::new(PortPoolInner {
                start,
                end,
                used: Mutex::new(HashSet::new()),
                next_hint: AtomicU16::new(start),
            }),
        }
    }

    /// 分配一对连续的 (RTP 偶数, RTCP 奇数) 端口
    pub fn allocate_pair(&self) -> Result<PortLease, MediaError> {
        let mut used = self.inner.used.lock();
        let total_ports = (self.inner.end - self.inner.start) / 2;

        let start_hint = self.inner.next_hint.load(Ordering::Relaxed);
        let mut port = start_hint;

        for _ in 0..total_ports {
            if port + 1 > self.inner.end {
                port = self.inner.start;
            }

            if !used.contains(&port) && !used.contains(&(port + 1)) {
                used.insert(port);
                used.insert(port + 1);

                let next = if port + 2 > self.inner.end {
                    self.inner.start
                } else {
                    port + 2
                };
                self.inner.next_hint.store(next, Ordering::Relaxed);

                return Ok(PortLease {
                    rtp_port: port,
                    rtcp_port: port + 1,
                    pool: self.inner.clone(),
                });
            }

            port += 2;
        }

        Err(MediaError::TooManyConsumers {
            max: total_ports as usize,
        })
    }

    /// 获取当前正在使用的端口对数量
    pub fn active_port_pairs(&self) -> usize {
        self.inner.used.lock().len() / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_pool_allocation_and_raii_release() {
        let pool = PortPool::new(30000, 30006);

        // 分配 1 对端口
        let lease1 = pool.allocate_pair().expect("alloc 1");
        assert_eq!(lease1.rtp_port, 30000);
        assert_eq!(lease1.rtcp_port, 30001);
        assert_eq!(pool.active_port_pairs(), 1);

        // 分配第 2 对端口
        let lease2 = pool.allocate_pair().expect("alloc 2");
        assert_eq!(lease2.rtp_port, 30002);
        assert_eq!(lease2.rtcp_port, 30003);
        assert_eq!(pool.active_port_pairs(), 2);

        // 释放 lease1
        drop(lease1);
        assert_eq!(pool.active_port_pairs(), 1);

        // 再次分配应复用释放的端口
        let lease3 = pool.allocate_pair().expect("alloc 3");
        assert_eq!(pool.active_port_pairs(), 2);
        drop(lease2);
        drop(lease3);
        assert_eq!(pool.active_port_pairs(), 0);
    }
}
