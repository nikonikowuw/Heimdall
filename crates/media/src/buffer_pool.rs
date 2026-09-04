use std::sync::atomic::{AtomicUsize, Ordering};

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
