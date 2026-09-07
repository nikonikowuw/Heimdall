use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;

use super::config::{DmaHeapCandidate, RgaCore, RgaPoolConfig};
use crate::error::AlgoError;

/// A DMA-BUF fd plus the allocation metadata needed by the RGA pool.
pub(crate) struct DmaBuffer {
    fd: OwnedFd,
    size: usize,
    dma32: bool,
}

impl DmaBuffer {
    pub(crate) fn fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    pub(crate) fn is_dma32(&self) -> bool {
        self.dma32
    }

    pub(crate) fn into_fd(self) -> OwnedFd {
        self.fd
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DmaAllocator {
    candidates: Vec<DmaHeapCandidate>,
    core: RgaCore,
}

impl DmaAllocator {
    pub(crate) fn from_config(config: &RgaPoolConfig) -> Self {
        Self {
            candidates: config.heap_candidates(),
            core: config.core,
        }
    }

    pub(crate) fn allocate(&self, size: usize) -> Result<DmaBuffer, AlgoError> {
        if size == 0 {
            return Err(AlgoError::OutOfMemory);
        }

        let mut errors = Vec::new();
        for candidate in &self.candidates {
            if !heap_allowed(self.core, candidate.dma32) {
                continue;
            }
            match allocate_from_heap(Path::new(&candidate.path), size) {
                Ok(fd) => {
                    return Ok(DmaBuffer {
                        fd,
                        size,
                        dma32: candidate.dma32,
                    });
                }
                Err(error) => errors.push(format!("{}: {error}", candidate.path)),
            }
        }
        if !errors.is_empty() {
            tracing::warn!(
                errors = %errors.join("; "),
                size,
                "failed to allocate DMA-BUF from candidate heaps"
            );
        }
        Err(AlgoError::OutOfMemory)
    }

    #[cfg(test)]
    pub(crate) fn candidate_paths(&self) -> Vec<&str> {
        self.candidates.iter().map(|c| c.path.as_str()).collect()
    }
}

fn heap_allowed(core: RgaCore, dma32: bool) -> bool {
    !matches!(core, RgaCore::Auto | RgaCore::Rga2) || dma32
}
fn allocate_from_heap(path: &Path, size: usize) -> std::io::Result<OwnedFd> {
    let heap = OpenOptions::new().read(true).write(true).open(path)?;
    super::ffi::allocate_dma_buf(heap.as_raw_fd(), size)
}

#[cfg(test)]
mod tests {
    use super::super::policy::RGA_DMA32_HEAP_PATH;
    use super::*;

    #[test]
    fn allocator_deduplicates_preferred_and_fallback_heaps() {
        let config = RgaPoolConfig::default();
        let allocator = DmaAllocator::from_config(&config);
        assert_eq!(
            allocator.candidate_paths().first().copied(),
            Some(RGA_DMA32_HEAP_PATH)
        );
        let unique = allocator
            .candidate_paths()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), allocator.candidate_paths().len());
        assert!(!super::heap_allowed(RgaCore::Rga2, false));
        assert!(super::heap_allowed(RgaCore::Rga2, true));
        assert!(super::heap_allowed(RgaCore::Rga3Core0, false));
    }
}
