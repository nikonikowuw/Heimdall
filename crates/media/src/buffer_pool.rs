use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
use crate::error::MediaError;

/// 帧缓冲区池状态监视
#[derive(Debug, Default)]
pub struct BufferPoolStats {
    pub total_allocated: AtomicUsize,
    pub in_use: AtomicUsize,
}

impl BufferPoolStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_in_use(&self) {
        self.in_use.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_in_use(&self) {
        self.in_use.fetch_sub(1, Ordering::Relaxed);
    }
}

/// DVPP 设备显存预分配池
///
/// 昇腾硬件架构下 `acldvppMalloc` 是内核/驱动级重型调用，
/// 通过预分配固定数量连续设备显存块，实现端到端全链路零动态分配，
/// 并通过 Condvar 实现租借背压与 LIFO 缓存亲和。
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct DvppBufferPool {
    /// 预分配的显存块地址列表（固定大小，不增长）
    blocks: Vec<*mut std::ffi::c_void>,
    /// 每块的字节大小
    block_size: usize,
    /// 可用缓冲区索引列表（LIFO for cache locality）
    available: Mutex<Vec<usize>>,
    /// 有缓冲区归还时通知等待中的解码线程
    not_empty: Condvar,
    /// 标记是否为测试/Mock 内存（用于单元测试跳过 acldvppFree）
    #[allow(unused)]
    is_mock: bool,
}

// SAFETY: blocks 指针指向全局统一编址的昇腾 Device Memory，
// 或在 mock 场景下指向由测试管理的有效内存；
// 租借与归还通过 Mutex 串行化保护，not_empty 条件变量保证唤醒同步，
// 满足 Send + Sync 约束。
unsafe impl Send for DvppBufferPool {}
// SAFETY: 跨线程并发访问通过内部 Mutex 保证线程安全。
unsafe impl Sync for DvppBufferPool {}

#[allow(dead_code)]
impl DvppBufferPool {
    /// 启动时预分配 `count` 个连续显存块（在 Linux DVPP 或测试环境下调用 acldvppMalloc）
    #[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
    pub fn new(block_size: usize, count: usize) -> Result<Self, MediaError> {
        let mut blocks: Vec<*mut std::ffi::c_void> = Vec::with_capacity(count);
        for _ in 0..count {
            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: 调用 AscendCL DVPP 内存分配函数，分配固定大小的连续设备显存
            let ret = unsafe { crate::decoders::dvpp::ffi::acldvppMalloc(&mut ptr, block_size) };
            if ret != 0 || ptr.is_null() {
                // 回滚释放此前已分配的所有块
                for &allocated in &blocks {
                    if !allocated.is_null() {
                        // SAFETY: 释放此前已成功分配的显存块
                        unsafe {
                            let _ = crate::decoders::dvpp::ffi::acldvppFree(allocated);
                        }
                    }
                }
                return Err(MediaError::DecoderInit {
                    codec: "H.264/H.265 (DVPP)".to_string(),
                    reason: format!("acldvppMalloc 分配 {block_size} 字节显存失败, code: {ret}"),
                });
            }
            blocks.push(ptr);
        }

        let available = Mutex::new((0..count).collect());
        Ok(Self {
            blocks,
            block_size,
            available,
            not_empty: Condvar::new(),
            is_mock: false,
        })
    }

    /// 创建 Mock 预分配显存池（供测试或无硬件环境验证池化调度）
    #[allow(unused)]
    pub fn new_mock(blocks: Vec<*mut std::ffi::c_void>, block_size: usize) -> Self {
        let count = blocks.len();
        let available = Mutex::new((0..count).collect());
        Self {
            blocks,
            block_size,
            available,
            not_empty: Condvar::new(),
            is_mock: true,
        }
    }

    /// 阻塞租借一个缓冲区（池空时阻塞挂起，直到有帧归还唤醒）
    pub fn acquire(&self) -> (*mut std::ffi::c_void, usize) {
        let mut avail = self.available.lock().unwrap_or_else(|p| p.into_inner());
        while avail.is_empty() {
            avail = self
                .not_empty
                .wait(avail)
                .unwrap_or_else(|p| p.into_inner());
        }
        let idx = avail.pop().expect("avail is not empty after condvar wait");
        (self.blocks[idx], self.block_size)
    }

    /// 带超时租借缓冲区（超时返回 None，防止流水线异常彻底死锁）
    pub fn acquire_timeout(&self, timeout: Duration) -> Option<(*mut std::ffi::c_void, usize)> {
        let mut avail = self.available.lock().unwrap_or_else(|p| p.into_inner());
        let start = std::time::Instant::now();
        while avail.is_empty() {
            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return None;
            }
            let remaining = timeout - elapsed;
            let (new_avail, wait_result) = self
                .not_empty
                .wait_timeout(avail, remaining)
                .unwrap_or_else(|p| p.into_inner());
            avail = new_avail;
            if wait_result.timed_out() && avail.is_empty() {
                return None;
            }
        }
        let idx = avail.pop()?;
        Some((self.blocks[idx], self.block_size))
    }

    /// 非阻塞尝试租借缓冲区（池空立即返回 None）
    pub fn try_acquire(&self) -> Option<(*mut std::ffi::c_void, usize)> {
        let mut avail = self.available.lock().unwrap_or_else(|p| p.into_inner());
        let idx = avail.pop()?;
        Some((self.blocks[idx], self.block_size))
    }

    /// 归还显存块至池（任意线程均可调用，由 FrameHandle 租约 Drop 触发）
    pub fn return_buffer(&self, ptr: *mut std::ffi::c_void) {
        let idx = self
            .blocks
            .iter()
            .position(|&p| p == ptr)
            .expect("归还了非本池管理的未知显存指针");
        let mut avail = self.available.lock().unwrap_or_else(|p| p.into_inner());
        avail.push(idx);
        self.not_empty.notify_one();
    }

    /// 当前池中可用空闲块数量
    pub fn available_count(&self) -> usize {
        self.available
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// 池总块数
    pub fn total_count(&self) -> usize {
        self.blocks.len()
    }

    /// 单块容量（字节）
    pub fn block_size(&self) -> usize {
        self.block_size
    }
}

impl Drop for DvppBufferPool {
    fn drop(&mut self) {
        #[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
        {
            if !self.is_mock {
                for &ptr in &self.blocks {
                    if !ptr.is_null() {
                        // SAFETY: 预分配的所有显存块在池析构时统一释放，
                        // 调用方保证此时上层流水线均已排空且无悬挂租约。
                        unsafe {
                            let _ = crate::decoders::dvpp::ffi::acldvppFree(ptr);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_dvpp_buffer_pool_lifo_and_exhaustion() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let dummy2 = 0x2000 as *mut std::ffi::c_void;
        let dummy3 = 0x3000 as *mut std::ffi::c_void;

        let pool = DvppBufferPool::new_mock(vec![dummy1, dummy2, dummy3], 1024);
        assert_eq!(pool.total_count(), 3);
        assert_eq!(pool.available_count(), 3);
        assert_eq!(pool.block_size(), 1024);

        // LIFO: last pushed is dummy3 (index 2)
        let (buf1, sz1) = pool.acquire();
        assert_eq!(buf1, dummy3);
        assert_eq!(sz1, 1024);
        assert_eq!(pool.available_count(), 2);

        let (buf2, _) = pool.acquire();
        assert_eq!(buf2, dummy2);

        let (buf3, _) = pool.acquire();
        assert_eq!(buf3, dummy1);
        assert_eq!(pool.available_count(), 0);

        // 池空时 try_acquire 返回 None
        assert!(pool.try_acquire().is_none());

        // acquire_timeout 也超时返回 None
        assert!(pool.acquire_timeout(Duration::from_millis(20)).is_none());

        // 归还一个 buffer
        pool.return_buffer(buf2);
        assert_eq!(pool.available_count(), 1);

        // 再次 acquire 成功取回刚刚归还的 buf2
        let (reacquired, _) = pool.acquire();
        assert_eq!(reacquired, buf2);
        assert_eq!(pool.available_count(), 0);
    }

    #[test]
    fn test_dvpp_buffer_pool_concurrent_wake() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = Arc::new(DvppBufferPool::new_mock(vec![dummy1], 512));

        // 先把唯一的一个 buffer 借走
        let (b, _) = pool.acquire();
        assert_eq!(pool.available_count(), 0);

        let pool_clone = Arc::clone(&pool);
        let b_addr = b as usize;
        let handle = thread::spawn(move || {
            // 稍后在另一个线程归还
            thread::sleep(Duration::from_millis(50));
            pool_clone.return_buffer(b_addr as *mut std::ffi::c_void);
        });

        // acquire 会阻塞并等待子线程归还被成功唤醒
        let start = std::time::Instant::now();
        let (woken_buf, sz) = pool.acquire();
        assert_eq!(woken_buf, dummy1);
        assert_eq!(sz, 512);
        assert!(start.elapsed() >= Duration::from_millis(30));

        handle.join().expect("线程正常结束");
    }
}
