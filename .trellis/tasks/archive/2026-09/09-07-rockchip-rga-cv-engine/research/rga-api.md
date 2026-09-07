# RGA implementation research

Date: 2026-02-14
Task: `09-07-rockchip-rga-cv-engine`

## Verified API facts

- Upstream `im2d_type.h` defines `rga_buffer_t` as a C layout containing virtual/physical pointers, fd, visible and stride dimensions, format, color-space mode, alpha, read mode, legacy fill color, and a `uint32_t` handle.
- The C-callable entry points needed here are `imcheck_t`, `improcess`, `imfill_t`, `wrapbuffer_handle_t`, and `releasebuffer_handle`.
- `importbuffer_fd` is exposed by some librga builds as a C++ overloaded symbol. The dynamic loader therefore needs to try both the plain symbol and the Itanium-mangled two-argument form (`_Z15importbuffer_fdii`).
- `IM_SYNC` is `1 << 19`; successful im2d calls use `IM_STATUS_SUCCESS` (1), while `imcheck_t` may return `IM_STATUS_NOERROR` (2).

Source: upstream `librga/include/im2d_type.h`, fetched from `https://raw.githubusercontent.com/airockchip/librga/master/include/im2d_type.h` on 2026-02-14.

## Hardware constraints

- RGA3 and RGA2 have different minimum dimensions, stride alignments, and scale-ratio limits. The Rust policy layer must reject unsupported jobs before entering the driver.
- Constant color fill is not universally available on RGA3; upstream issue reports show `imfill`/constant border operations may require RGA2 and DMA32-addressable memory on RK3588-class systems. Letterbox fill is consequently an explicit operation whose driver result is checked; the implementation does not claim that every RGA core supports it.
- RGA handles are imported once per long-lived DMA-BUF pool slot and released only when the slot is evicted or the pool is dropped. A source handle is scoped to the source frame and released after synchronous processing.

Source: `https://github.com/airockchip/librga/issues/143` and the upstream header above. These driver-specific conclusions remain unverified on the RK3576 board until matching `librga`, kernel, and RGA driver versions are tested.

## Implementation notes

- `RgaBufferPool` performs idle eviction lazily at the next `acquire`; it does not create an unbounded background reaper thread. Dropping the pool still releases every remaining imported handle and DMA-BUF through RAII.
- `RgaCore::Auto` and `RgaCore::Rga2` require an allocation reported as coming from a `dma32` heap. If only a generic heap is available, allocation is rejected rather than silently claiming RGA2 4GB safety. `RgaCore::Rga3Core0` and `RgaCore::Rga3Core1` select the RGA3 validation profile only; actual scheduler-core forcing is intentionally left disabled until board/driver validation.
- Because `rga_buffer_t` exposes one image stride rather than independent per-plane strides, the hardware path accepts only shared NV12 Y/UV stride and the canonical I420 relation `y_stride = 2 * u_stride = 2 * v_stride`; other valid `SafeFrame` layouts remain available to the CPU/host engines but are rejected by the DMA-BUF RGA path.
- `rga` is a Linux-only implementation feature at the module boundary. The `tracing` dependency is optional and enabled only with that feature.


The development host is x86_64 Linux and has no local Rockchip RGA headers or shared library. The implementation therefore uses runtime loading and pure-logic/mock tests; actual DMA-heap allocation, handle import, stride behavior, color fill, and driver synchronization require board validation.
