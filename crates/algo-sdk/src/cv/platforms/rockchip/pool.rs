use std::collections::VecDeque;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::config::{RgaCore, RgaPoolConfig};
use super::dma_alloc::DmaAllocator;
use super::ffi::RgaRuntime;
use super::policy::{align_up, checked_rgb_bytes, RgaPolicy};
use crate::cv::types::PixelFormat;
use crate::error::AlgoError;

const RGA_FORMAT_RGB_888: u32 = 0x200;

/// Immutable geometry of one output pool. Strides are in pixels, as required by librga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgaBufferSpec {
    pub width: u32,
    pub height: u32,
    pub w_stride: u32,
    pub h_stride: u32,
    pub format: PixelFormat,
    pub size: usize,
}

impl RgaBufferSpec {
    pub(crate) fn from_config(config: &RgaPoolConfig) -> Result<Self, AlgoError> {
        let policy = RgaPolicy::new(config.core);
        let w_stride = align_up(config.width, policy.stride_alignment(config.format))
            .ok_or(AlgoError::OutOfMemory)?;
        let h_stride = align_up(config.height, 2).ok_or(AlgoError::OutOfMemory)?;
        policy.validate_output(
            config.width,
            config.height,
            w_stride,
            h_stride,
            config.format,
        )?;
        let size = checked_rgb_bytes(w_stride, h_stride)?;
        Ok(Self {
            width: config.width,
            height: config.height,
            w_stride,
            h_stride,
            format: config.format,
            size,
        })
    }
}

pub(crate) trait SlotFactory: Send + Sync {
    fn create(&self) -> Result<SlotData, AlgoError>;
    fn release(&self, handle: u32);
}

pub(crate) struct SlotData {
    pub fd: OwnedFd,
    pub handle: u32,
}

struct PoolResource {
    fd: OwnedFd,
    handle: u32,
    releaser: Arc<dyn SlotFactory>,
}

impl PoolResource {
    fn new(data: SlotData, releaser: Arc<dyn SlotFactory>) -> Self {
        Self {
            fd: data.fd,
            handle: data.handle,
            releaser,
        }
    }
}

impl Drop for PoolResource {
    fn drop(&mut self) {
        self.releaser.release(self.handle);
    }
}

struct RgaSlotFactory {
    runtime: Arc<RgaRuntime>,
    allocator: DmaAllocator,
    spec: RgaBufferSpec,
    core: RgaCore,
}

impl SlotFactory for RgaSlotFactory {
    fn create(&self) -> Result<SlotData, AlgoError> {
        let dma = self.allocator.allocate(self.spec.size)?;
        RgaPolicy::new(self.core).validate_dma_allocation(dma.is_dma32())?;
        let handle = self.runtime.import_buffer_fd(
            dma.fd(),
            dma.size(),
            self.spec.w_stride,
            self.spec.h_stride,
            RGA_FORMAT_RGB_888,
        )?;
        Ok(SlotData {
            fd: dma.into_fd(),
            handle,
        })
    }

    fn release(&self, handle: u32) {
        if let Err(error) = self.runtime.release_buffer_handle(handle) {
            tracing::warn!(handle, %error, "failed to release pooled RGA handle");
        }
    }
}

struct PoolSlot {
    resource: PoolResource,
    leased: bool,
    last_used: Instant,
}

struct PoolState {
    slots: Vec<Option<PoolSlot>>,
    idle: VecDeque<usize>,
    creating: usize,
}

impl PoolState {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            idle: VecDeque::new(),
            creating: 0,
        }
    }

    fn slot_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    fn idle_count(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| !slot.leased)
            .count()
    }

    fn insert(&mut self, slot: PoolSlot) -> usize {
        if let Some(index) = self.slots.iter().position(Option::is_none) {
            self.slots[index] = Some(slot);
            index
        } else {
            self.slots.push(Some(slot));
            self.slots.len() - 1
        }
    }

    fn take_idle(&mut self) -> Option<(usize, i32, u32)> {
        while let Some(index) = self.idle.pop_front() {
            if let Some(slot) = self.slots.get_mut(index).and_then(Option::as_mut) {
                if !slot.leased {
                    slot.leased = true;
                    return Some((index, slot.resource.fd.as_raw_fd(), slot.resource.handle));
                }
            }
        }
        None
    }

    fn evict_expired(&mut self, min_idle: usize, idle_timeout: Duration) -> Vec<PoolResource> {
        let now = Instant::now();
        let mut candidates = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                slot.as_ref().filter(|slot| {
                    !slot.leased && now.saturating_duration_since(slot.last_used) >= idle_timeout
                })?;
                Some(index)
            })
            .collect::<Vec<_>>();
        let evict_count = self.idle_count().saturating_sub(min_idle);
        candidates.truncate(evict_count);

        let mut retired = Vec::with_capacity(candidates.len());
        for index in candidates {
            if let Some(slot) = self.slots[index].take() {
                retired.push(slot.resource);
            }
        }
        self.idle.retain(|index| {
            self.slots
                .get(*index)
                .and_then(Option::as_ref)
                .is_some_and(|slot| !slot.leased)
        });
        retired
    }
}

struct PoolInner {
    config: RgaPoolConfig,
    factory: Arc<dyn SlotFactory>,
    state: Mutex<PoolState>,
    wake: Condvar,
}

impl PoolInner {
    fn create_resource(&self) -> Result<PoolResource, AlgoError> {
        let data = self.factory.create()?;
        Ok(PoolResource::new(data, Arc::clone(&self.factory)))
    }
}

/// Bounded, reusable DMA-BUF/RGA-handle pool.
#[derive(Clone)]
pub struct RgaBufferPool {
    inner: Arc<PoolInner>,
}

impl std::fmt::Debug for RgaBufferPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RgaBufferPool")
            .field("stats", &self.stats())
            .finish()
    }
}

impl RgaBufferPool {
    /// Construct a pool using the process-local dynamically loaded librga runtime.
    pub fn new(config: RgaPoolConfig) -> Result<Self, AlgoError> {
        config.validate()?;
        let runtime = RgaRuntime::load()?;
        Self::with_runtime(config, runtime)
    }

    pub(crate) fn with_runtime(
        config: RgaPoolConfig,
        runtime: Arc<RgaRuntime>,
    ) -> Result<Self, AlgoError> {
        config.validate()?;
        let spec = RgaBufferSpec::from_config(&config)?;
        let factory: Arc<dyn SlotFactory> = Arc::new(RgaSlotFactory {
            runtime,
            allocator: DmaAllocator::from_config(&config),
            spec,
            core: config.core,
        });
        Self::with_factory(config, factory)
    }

    pub(crate) fn with_factory(
        config: RgaPoolConfig,
        factory: Arc<dyn SlotFactory>,
    ) -> Result<Self, AlgoError> {
        config.validate()?;
        let inner = Arc::new(PoolInner {
            config: config.clone(),
            factory,
            state: Mutex::new(PoolState::new()),
            wake: Condvar::new(),
        });
        let pool = Self { inner };
        for _ in 0..config.min_idle {
            let resource = pool.inner.create_resource()?;
            let mut state = pool.inner.state.lock().map_err(|_| AlgoError::Internal {
                reason: "RGA pool lock poisoned".to_string(),
            })?;
            let index = state.insert(PoolSlot {
                resource,
                leased: false,
                last_used: Instant::now(),
            });
            state.idle.push_back(index);
        }
        Ok(pool)
    }

    pub fn acquire(&self) -> Result<PooledRgaBuffer, AlgoError> {
        let deadline = Instant::now().checked_add(self.inner.config.acquire_timeout());
        loop {
            let mut retired = Vec::new();
            let mut state = self.inner.state.lock().map_err(|_| AlgoError::Internal {
                reason: "RGA pool lock poisoned".to_string(),
            })?;
            retired.extend(
                state.evict_expired(self.inner.config.min_idle, self.inner.config.idle_timeout()),
            );
            if let Some((slot_id, fd, handle)) = state.take_idle() {
                drop(state);
                drop(retired);
                return Ok(PooledRgaBuffer {
                    slot_id,
                    fd,
                    handle,
                    pool: Arc::clone(&self.inner),
                });
            }

            if state.slot_count() + state.creating < self.inner.config.max_size {
                state.creating += 1;
                drop(state);
                drop(retired);
                let created = self.inner.create_resource();
                let mut state = self.inner.state.lock().map_err(|_| AlgoError::Internal {
                    reason: "RGA pool lock poisoned".to_string(),
                })?;
                state.creating = state.creating.saturating_sub(1);
                match created {
                    Ok(resource) => {
                        let fd = resource.fd.as_raw_fd();
                        let handle = resource.handle;
                        let slot_id = state.insert(PoolSlot {
                            resource,
                            leased: true,
                            last_used: Instant::now(),
                        });
                        self.inner.wake.notify_all();
                        drop(state);
                        return Ok(PooledRgaBuffer {
                            slot_id,
                            fd,
                            handle,
                            pool: Arc::clone(&self.inner),
                        });
                    }
                    Err(error) => {
                        self.inner.wake.notify_all();
                        return Err(error);
                    }
                }
            }

            let Some(remaining) = deadline
                .and_then(|d| d.checked_duration_since(Instant::now()))
                .filter(|r| !r.is_zero())
            else {
                drop(state);
                drop(retired);
                return Err(AlgoError::Timeout);
            };
            let (next_state, result) =
                self.inner
                    .wake
                    .wait_timeout(state, remaining)
                    .map_err(|_| AlgoError::Internal {
                        reason: "RGA pool lock poisoned".to_string(),
                    })?;
            drop(next_state);
            drop(retired);
            if result.timed_out() {
                return Err(AlgoError::Timeout);
            }
        }
    }

    pub fn stats(&self) -> RgaPoolStats {
        let Ok(state) = self.inner.state.lock() else {
            return RgaPoolStats::default();
        };
        RgaPoolStats {
            total: state.slot_count(),
            idle: state.idle_count(),
            leased: state.slot_count().saturating_sub(state.idle_count()),
            max_size: self.inner.config.max_size,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RgaPoolStats {
    pub total: usize,
    pub idle: usize,
    pub leased: usize,
    pub max_size: usize,
}

/// RAII lease for one pool slot. Dropping it returns the slot without releasing its RGA handle.
pub struct PooledRgaBuffer {
    slot_id: usize,
    fd: i32,
    handle: u32,
    pool: Arc<PoolInner>,
}

impl std::fmt::Debug for PooledRgaBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PooledRgaBuffer")
            .field("slot_id", &self.slot_id)
            .field("fd", &self.fd)
            .field("handle", &self.handle)
            .finish()
    }
}

impl PooledRgaBuffer {
    pub fn fd(&self) -> i32 {
        self.fd
    }

    pub fn handle(&self) -> u32 {
        self.handle
    }
}

impl Drop for PooledRgaBuffer {
    fn drop(&mut self) {
        let Ok(mut state) = self.pool.state.lock() else {
            tracing::error!(
                slot_id = self.slot_id,
                "RGA pool lock poisoned while recycling slot"
            );
            return;
        };
        if let Some(slot) = state.slots.get_mut(self.slot_id).and_then(Option::as_mut) {
            if slot.leased {
                slot.leased = false;
                slot.last_used = Instant::now();
                state.idle.push_back(self.slot_id);
                self.pool.wake.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::fd::{FromRawFd, IntoRawFd};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use super::*;

    struct MockFactory {
        next_handle: AtomicU32,
        creates: AtomicUsize,
        releases: AtomicUsize,
    }

    impl MockFactory {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                next_handle: AtomicU32::new(1),
                creates: AtomicUsize::new(0),
                releases: AtomicUsize::new(0),
            })
        }
    }

    impl SlotFactory for MockFactory {
        fn create(&self) -> Result<SlotData, AlgoError> {
            let file = File::open("/dev/null").map_err(|_| AlgoError::OutOfMemory)?;
            let fd = file.into_raw_fd();
            // SAFETY: into_raw_fd transfers this newly-opened /dev/null fd to OwnedFd.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
            self.creates.fetch_add(1, Ordering::Relaxed);
            Ok(SlotData { fd, handle })
        }

        fn release(&self, _handle: u32) {
            self.releases.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn test_config(min_idle: usize, max_size: usize) -> RgaPoolConfig {
        RgaPoolConfig {
            min_idle,
            max_size,
            idle_timeout_sec: 60,
            acquire_timeout_ms: 5,
            ..RgaPoolConfig::default()
        }
    }

    fn test_pool(config: RgaPoolConfig, factory: Arc<MockFactory>) -> RgaBufferPool {
        RgaBufferPool::with_factory(config, factory).expect("mock pool")
    }

    #[test]
    fn test_rga_pool_lease_and_recycle() {
        let factory = MockFactory::new();
        let pool = test_pool(test_config(2, 4), Arc::clone(&factory));
        assert_eq!(factory.creates.load(Ordering::Relaxed), 2);
        let lease = pool.acquire().expect("lease");
        let first_handle = lease.handle();
        drop(lease);
        let second = pool.acquire().expect("second lease");
        let second_handle = second.handle();
        drop(second);
        let third = pool.acquire().expect("reused lease");
        assert!(third.handle() == first_handle || third.handle() == second_handle);
        assert_eq!(factory.creates.load(Ordering::Relaxed), 2);
        assert_eq!(pool.stats().total, 2);
        drop(third);
        drop(pool);
        assert_eq!(factory.releases.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_rga_pool_timeout_protection() {
        let factory = MockFactory::new();
        let pool = test_pool(test_config(0, 1), factory);
        let _lease = pool.acquire().expect("first lease");
        assert!(matches!(pool.acquire(), Err(AlgoError::Timeout)));
    }

    #[test]
    fn test_rga_pool_burst_and_idle_eviction() {
        let factory = MockFactory::new();
        let mut config = test_config(1, 3);
        config.idle_timeout_sec = 0;
        let pool = test_pool(config, Arc::clone(&factory));
        let first = pool.acquire().expect("first");
        let second = pool.acquire().expect("second");
        let third = pool.acquire().expect("third");
        drop(first);
        drop(second);
        drop(third);
        let _lease = pool.acquire().expect("after shrink");
        assert_eq!(pool.stats().total, 1);
        assert_eq!(pool.stats().idle, 0);
        drop(_lease);
        drop(pool);
        assert_eq!(factory.releases.load(Ordering::Relaxed), 3);
    }
}
