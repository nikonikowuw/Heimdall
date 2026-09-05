use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use thiserror::Error;

#[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
use crate::error::MediaError;

/// 全局显存池代际单调递增计数器（每次新池初始化或动态重构时递增）
static NEXT_POOL_GENERATION: AtomicU64 = AtomicU64::new(1);

/// 显存池生命周期与流转异常（支持安全降级与故障隔离，严禁导致无人值守进程 panic）
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PoolError {
    #[error("归还了非本显存池管理的未知指针: {0:?}")]
    UnknownPointer(*mut std::ffi::c_void),

    #[error("检测到显存块重复归还 (Double Return 防护生效): {0:?}")]
    DuplicateReturn(*mut std::ffi::c_void),

    #[error("显存池已关闭或正在析构，拒绝接收归还")]
    PoolClosed,

    #[error("显存池代际不匹配 (当前池代际: {current}, 归还租约代际: {returned})")]
    GenerationMismatch { current: u64, returned: u64 },
}

/// 显存池全链路健康诊断指标（无锁原子计数，支持监控与故障分析）
#[derive(Debug, Default)]
pub struct PoolDiagnostics {
    /// 租借成功总次数
    pub acquire_count: AtomicU64,
    /// 成功归还总次数
    pub successful_return_count: AtomicU64,
    /// 未知指针拦截次数（unknown pointer）
    pub unknown_pointer_count: AtomicU64,
    /// 重复归还拦截次数 (double return)
    pub duplicate_return_count: AtomicU64,
    /// 池关闭后归还拦截次数 (pool closed return)
    pub pool_closed_return_count: AtomicU64,
    /// 代际不匹配拦截次数 (generation mismatch)
    pub generation_mismatch_count: AtomicU64,
}

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

/// 互斥锁保护的池核心状态
#[derive(Debug)]
struct PoolInner {
    /// 可用缓冲区索引列表（LIFO for cache locality）
    available: Vec<usize>,
    /// 每块当前的租借占用状态，len == blocks.len()
    /// true 表示该块正在被租借使用中，false 表示该块在空闲池中
    in_use: Vec<bool>,
    /// 是否已关闭
    is_closed: bool,
}

/// DVPP 设备显存预分配池
///
/// 昇腾硬件架构下 `acldvppMalloc` 是内核/驱动级重型调用，
/// 通过预分配固定数量连续设备显存块，实现端到端全链路零动态分配，
/// 并通过 Condvar 实现租借背压与 LIFO 缓存亲和。
/// 全链路杜绝 panic，对未知指针、重复归还、代际错乱及池关闭实行防御性拦截与指标审计。
#[derive(Debug)]
pub(crate) struct DvppBufferPool {
    /// 显存池代际版本（每次重建或重配递增，防止代际混淆）
    generation: u64,
    /// 预分配的显存块地址列表（固定大小，不增长）
    blocks: Vec<*mut std::ffi::c_void>,
    /// 每块的字节大小
    block_size: usize,
    /// 状态追踪互斥锁（原子管理 available、in_use 与 is_closed）
    inner: Mutex<PoolInner>,
    /// 有缓冲区归还时通知等待中的解码线程
    not_empty: Condvar,
    /// 健康指标计数器
    diagnostics: PoolDiagnostics,
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
        let generation = NEXT_POOL_GENERATION.fetch_add(1, Ordering::Relaxed);
        Self::new_with_generation(block_size, count, generation)
    }

    /// 指定代际预分配显存池
    #[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
    pub fn new_with_generation(
        block_size: usize,
        count: usize,
        generation: u64,
    ) -> Result<Self, MediaError> {
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

        let inner = Mutex::new(PoolInner {
            available: (0..count).collect(),
            in_use: vec![false; count],
            is_closed: false,
        });
        Ok(Self {
            generation,
            blocks,
            block_size,
            inner,
            not_empty: Condvar::new(),
            diagnostics: PoolDiagnostics::default(),
            is_mock: false,
        })
    }

    /// 创建 Mock 预分配显存池（供测试或无硬件环境验证池化调度）
    #[allow(unused)]
    pub fn new_mock(blocks: Vec<*mut std::ffi::c_void>, block_size: usize) -> Self {
        let generation = NEXT_POOL_GENERATION.fetch_add(1, Ordering::Relaxed);
        Self::new_mock_with_generation(blocks, block_size, generation)
    }

    /// 指定代际创建 Mock 预分配显存池
    #[allow(unused)]
    pub fn new_mock_with_generation(
        blocks: Vec<*mut std::ffi::c_void>,
        block_size: usize,
        generation: u64,
    ) -> Self {
        let count = blocks.len();
        let inner = Mutex::new(PoolInner {
            available: (0..count).collect(),
            in_use: vec![false; count],
            is_closed: false,
        });
        Self {
            generation,
            blocks,
            block_size,
            inner,
            not_empty: Condvar::new(),
            diagnostics: PoolDiagnostics::default(),
            is_mock: true,
        }
    }

    /// 阻塞租借一个缓冲区（池空时阻塞挂起，直到有帧归还唤醒；池关闭时返回 None）
    pub fn acquire(&self) -> Option<(*mut std::ffi::c_void, usize)> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        while inner.available.is_empty() {
            if inner.is_closed {
                return None;
            }
            inner = self
                .not_empty
                .wait(inner)
                .unwrap_or_else(|p| p.into_inner());
        }
        if inner.is_closed {
            return None;
        }
        let idx = inner.available.pop()?;
        inner.in_use[idx] = true;
        self.diagnostics
            .acquire_count
            .fetch_add(1, Ordering::Relaxed);
        Some((self.blocks[idx], self.block_size))
    }

    /// 带超时租借缓冲区（超时或池关闭返回 None，防止流水线异常彻底死锁）
    pub fn acquire_timeout(&self, timeout: Duration) -> Option<(*mut std::ffi::c_void, usize)> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let start = std::time::Instant::now();
        while inner.available.is_empty() {
            if inner.is_closed {
                return None;
            }
            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return None;
            }
            let remaining = timeout - elapsed;
            let (new_inner, wait_result) = self
                .not_empty
                .wait_timeout(inner, remaining)
                .unwrap_or_else(|p| p.into_inner());
            inner = new_inner;
            if inner.is_closed {
                return None;
            }
            if wait_result.timed_out() && inner.available.is_empty() {
                return None;
            }
        }
        let idx = inner.available.pop()?;
        inner.in_use[idx] = true;
        self.diagnostics
            .acquire_count
            .fetch_add(1, Ordering::Relaxed);
        Some((self.blocks[idx], self.block_size))
    }

    /// 非阻塞尝试租借缓冲区（池空或池关闭立即返回 None）
    pub fn try_acquire(&self) -> Option<(*mut std::ffi::c_void, usize)> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.is_closed {
            return None;
        }
        let idx = inner.available.pop()?;
        inner.in_use[idx] = true;
        self.diagnostics
            .acquire_count
            .fetch_add(1, Ordering::Relaxed);
        Some((self.blocks[idx], self.block_size))
    }

    /// 归还显存块至池（带代际校验，防御旧池租约延迟归还）
    pub fn return_buffer_with_generation(
        &self,
        ptr: *mut std::ffi::c_void,
        returned_generation: u64,
    ) -> Result<(), PoolError> {
        if returned_generation != self.generation {
            tracing::warn!(
                current_generation = self.generation,
                returned_generation,
                ptr = ?ptr,
                "DvppBufferPool: 显存块归还代际不匹配 (可能是旧池残留租约延迟释放)"
            );
            self.diagnostics
                .generation_mismatch_count
                .fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::GenerationMismatch {
                current: self.generation,
                returned: returned_generation,
            });
        }
        self.return_buffer(ptr)
    }

    /// 归还显存块至池（任意线程均可安全调用，严格防御 panic、内存破坏与二次入队）
    pub fn return_buffer(&self, ptr: *mut std::ffi::c_void) -> Result<(), PoolError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());

        // 1. 池关闭状态检查
        if inner.is_closed {
            tracing::warn!(
                ptr = ?ptr,
                generation = self.generation,
                "DvppBufferPool: 池已处于关闭/析构状态，拒绝接收归还显存块"
            );
            self.diagnostics
                .pool_closed_return_count
                .fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::PoolClosed);
        }

        // 2. 未知指针检查（Unknown Pointer 防护）
        let idx = match self.blocks.iter().position(|&p| p == ptr) {
            Some(i) => i,
            None => {
                tracing::warn!(
                    ptr = ?ptr,
                    generation = self.generation,
                    "DvppBufferPool: 尝试归还非本池管理的未知显存指针"
                );
                self.diagnostics
                    .unknown_pointer_count
                    .fetch_add(1, Ordering::Relaxed);
                return Err(PoolError::UnknownPointer(ptr));
            }
        };

        // 3. 重复归还检查（Double Return 防护）
        if !inner.in_use[idx] {
            tracing::error!(
                ptr = ?ptr,
                idx,
                generation = self.generation,
                "DvppBufferPool: 拦截到显存块重复归还 (Double Return 防护生效，拒绝二次入队)"
            );
            self.diagnostics
                .duplicate_return_count
                .fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::DuplicateReturn(ptr));
        }

        // 4. 正常归还：标记为空闲并压入可用队列
        inner.in_use[idx] = false;
        inner.available.push(idx);
        self.diagnostics
            .successful_return_count
            .fetch_add(1, Ordering::Relaxed);
        self.not_empty.notify_one();
        Ok(())
    }

    /// 安全关闭显存池（唤醒所有挂起等待者，防止线程挂起或死锁）
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !inner.is_closed {
            inner.is_closed = true;
            self.not_empty.notify_all();
        }
    }

    /// 检查显存池是否已关闭
    pub fn is_closed(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_closed
    }

    /// 获取池代际号
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 获取全链路诊断统计指标快照引用
    pub fn diagnostics(&self) -> &PoolDiagnostics {
        &self.diagnostics
    }

    /// 当前池中可用空闲块数量
    pub fn available_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .available
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
        self.close();
        #[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
        {
            if !self.is_mock {
                for &ptr in &self.blocks {
                    if !ptr.is_null() {
                        // SAFETY: 预分配的所有显存块在池析构时统一释放
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
        let (buf1, sz1) = pool.acquire().expect("acquire 成功");
        assert_eq!(buf1, dummy3);
        assert_eq!(sz1, 1024);
        assert_eq!(pool.available_count(), 2);

        let (buf2, _) = pool.acquire().expect("acquire 成功");
        assert_eq!(buf2, dummy2);

        let (buf3, _) = pool.acquire().expect("acquire 成功");
        assert_eq!(buf3, dummy1);
        assert_eq!(pool.available_count(), 0);

        // 池空时 try_acquire 返回 None
        assert!(pool.try_acquire().is_none());

        // acquire_timeout 也超时返回 None
        assert!(pool.acquire_timeout(Duration::from_millis(20)).is_none());

        // 归还一个 buffer
        let ret_res = pool.return_buffer(buf2);
        assert!(ret_res.is_ok(), "正常归还应当返回 Ok");
        assert_eq!(pool.available_count(), 1);

        // 再次 acquire 成功取回刚刚归还的 buf2
        let (reacquired, _) = pool.acquire().expect("acquire 成功");
        assert_eq!(reacquired, buf2);
        assert_eq!(pool.available_count(), 0);
    }

    #[test]
    fn test_dvpp_buffer_pool_concurrent_wake() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = Arc::new(DvppBufferPool::new_mock(vec![dummy1], 512));

        // 先把唯一的一个 buffer 借走
        let (b, _) = pool.acquire().expect("acquire 成功");
        assert_eq!(pool.available_count(), 0);

        let pool_clone = Arc::clone(&pool);
        let b_addr = b as usize;
        let handle = thread::spawn(move || {
            // 稍后在另一个线程归还
            thread::sleep(Duration::from_millis(50));
            let res = pool_clone.return_buffer(b_addr as *mut std::ffi::c_void);
            assert!(res.is_ok(), "并发归还应当返回 Ok");
        });

        // acquire 会阻塞并等待子线程归还被成功唤醒
        let start = std::time::Instant::now();
        let (woken_buf, sz) = pool.acquire().expect("唤醒 acquire 成功");
        assert_eq!(woken_buf, dummy1);
        assert_eq!(sz, 512);
        assert!(start.elapsed() >= Duration::from_millis(30));

        handle.join().expect("线程正常结束");
    }

    #[test]
    fn test_dvpp_buffer_pool_unknown_pointer_defense() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = DvppBufferPool::new_mock(vec![dummy1], 512);

        let unknown = 0x9999 as *mut std::ffi::c_void;
        let res = pool.return_buffer(unknown);

        assert!(res.is_err(), "未知指针必须返回 Err 而不是 panic");
        assert_eq!(
            res.expect_err("应当为 Err"),
            PoolError::UnknownPointer(unknown)
        );
        assert_eq!(
            pool.diagnostics()
                .unknown_pointer_count
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn test_dvpp_buffer_pool_double_return_defense() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = DvppBufferPool::new_mock(vec![dummy1], 512);

        let (b, _) = pool.acquire().expect("acquire 成功");
        assert_eq!(pool.available_count(), 0);

        // 第一次正常归还
        let res1 = pool.return_buffer(b);
        assert!(res1.is_ok());
        assert_eq!(pool.available_count(), 1);

        // 第二次重复归还 (Double Return)
        let res2 = pool.return_buffer(b);
        assert!(res2.is_err(), "重复归还必须拦截拒收");
        assert_eq!(res2.expect_err("应当为 Err"), PoolError::DuplicateReturn(b));
        // 关键断言：池内仍然只有 1 个可用块，绝不造成二次入队数据竞争！
        assert_eq!(pool.available_count(), 1);
        assert_eq!(
            pool.diagnostics()
                .duplicate_return_count
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn test_dvpp_buffer_pool_closed_defense() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = DvppBufferPool::new_mock(vec![dummy1], 512);

        let (b, _) = pool.acquire().expect("acquire 成功");
        pool.close();
        assert!(pool.is_closed());

        // 关闭后 acquire 立即返回 None
        assert!(pool.acquire().is_none());
        assert!(pool.try_acquire().is_none());

        // 关闭后归还应安全拒绝，返回 PoolClosed 且不 panic
        let res = pool.return_buffer(b);
        assert!(res.is_err());
        assert_eq!(res.expect_err("应当为 Err"), PoolError::PoolClosed);
        assert_eq!(
            pool.diagnostics()
                .pool_closed_return_count
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn test_dvpp_buffer_pool_generation_mismatch_defense() {
        let dummy1 = 0x1000 as *mut std::ffi::c_void;
        let pool = DvppBufferPool::new_mock_with_generation(vec![dummy1], 512, 10);

        let (b, _) = pool.acquire().expect("acquire 成功");

        // 使用旧代际 9 归还
        let res_old = pool.return_buffer_with_generation(b, 9);
        assert!(res_old.is_err(), "代际不匹配必须被拦截");
        match res_old.expect_err("应当为 Err") {
            PoolError::GenerationMismatch { current, returned } => {
                assert_eq!(current, 10);
                assert_eq!(returned, 9);
            }
            other => panic!("预期 GenerationMismatch, 实际: {:?}", other),
        }
        assert_eq!(
            pool.diagnostics()
                .generation_mismatch_count
                .load(Ordering::Relaxed),
            1
        );

        // 使用正确代际 10 归还
        let res_cur = pool.return_buffer_with_generation(b, 10);
        assert!(res_cur.is_ok(), "匹配代际应当成功归还");
        assert_eq!(pool.available_count(), 1);
    }
}
