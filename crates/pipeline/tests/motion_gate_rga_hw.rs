//! 运动门控 DMA-BUF 硬件链路验证（Rockchip RGA）
//!
//! 回归目标：`FrameHandle::DmaBuf`（硬解帧的唯一生产载体）必须真正参与运动门控判定。
//! 修复前该分支被静默放行，静止画面也会全速推给 NPU。
//!
//! 需要 `/dev/rga`、librga 与 `/dev/dma_heap`，无法在开发机运行，板端显式执行：
//!
//! ```bash
//! cargo test -p pipeline --features rga --test motion_gate_rga_hw -- --ignored
//! ```
#![cfg(all(target_os = "linux", feature = "rga"))]

use std::os::fd::{AsRawFd, OwnedFd};

use media::dmabuf_sync::alloc_dma_buf;
use pipeline::MotionGate;
use types::{FrameHandle, FrameRef, MotionGateConfig, PixelFormat, StrideInfo};

const SRC_W: u32 = 1920;
const SRC_H: u32 = 1080;

/// 分配一张 NV12 DMA-BUF 源帧，Y 平面填充指定灰度、色度填充 128
fn alloc_source_frame(luma: u8) -> OwnedFd {
    let len = (SRC_W as usize) * (SRC_H as usize) * 3 / 2;
    let fd = alloc_dma_buf(len).expect("分配源 DMA-BUF");
    let raw_fd = fd.as_raw_fd();

    // SAFETY: 基于具有生命周期的 OwnedFd 建立读写共享映射，长度为 len
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            raw_fd,
            0,
        )
    };
    assert_ne!(ptr, libc::MAP_FAILED, "源 DMA-BUF mmap 失败");

    // SAFETY: ptr 为刚建立的合法映射，长度 len（Y 平面 + UV 平面）
    unsafe {
        std::ptr::write_bytes(ptr as *mut u8, luma, (SRC_W as usize) * (SRC_H as usize));
        std::ptr::write_bytes(
            (ptr as *mut u8).add((SRC_W as usize) * (SRC_H as usize)),
            128,
            len - (SRC_W as usize) * (SRC_H as usize),
        );
        libc::munmap(ptr, len);
    }

    fd
}

/// 在源帧 Y 平面上写入一块高亮矩形，模拟真实运动
fn paint_motion_block(fd: &OwnedFd, luma: u8) {
    let y_len = (SRC_W as usize) * (SRC_H as usize);
    let raw_fd = fd.as_raw_fd();

    // SAFETY: 基于具有生命周期的 OwnedFd 建立读写共享映射，长度为 y_len
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            y_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            raw_fd,
            0,
        )
    };
    assert_ne!(ptr, libc::MAP_FAILED, "源 DMA-BUF mmap 失败");

    // 中央 480x270 区域（缩略图上约 80x45 像素，远超默认 contour_area=100）
    for y in (SRC_H as usize / 4)..(SRC_H as usize * 3 / 4) {
        // SAFETY: 行偏移落在映射范围内
        unsafe {
            std::ptr::write_bytes((ptr as *mut u8).add(y * SRC_W as usize + 720), luma, 480);
        }
    }

    // SAFETY: 解除临时映射
    unsafe { libc::munmap(ptr, y_len) };
}

fn frame_from(fd: OwnedFd, timestamp: i64) -> FrameRef {
    FrameRef::new(
        "cam_rga_hw".to_string(),
        timestamp,
        SRC_W,
        SRC_H,
        StrideInfo::new(SRC_W, SRC_H),
        PixelFormat::Nv12,
        FrameHandle::DmaBuf {
            fd: std::sync::Arc::new(fd),
            _lease: None,
        },
    )
}

#[test]
#[ignore = "需要 Rockchip RGA 硬件与 /dev/dma_heap"]
fn dma_buf_frames_are_actually_gated_by_rga_thumbnail() {
    let mut gate = MotionGate::new(MotionGateConfig {
        enabled: true,
        threshold: 25,
        contour_area: 100,
        keepalive_interval_ms: 60_000,
        motion_hold_frames: 0,
    });

    let fd = alloc_source_frame(100);
    let frame = frame_from(fd, 1000);
    // 运动场景需要重新映射同一张 DMA-BUF 写入像素
    let source_fd = match frame.handle() {
        FrameHandle::DmaBuf { fd, .. } => fd.clone(),
        other => panic!("需要 DMA-BUF 帧载体，实际: {other:?}"),
    };

    // 1. 首帧建立参考背景：无参考帧放行 + 保活
    let first = gate.evaluate_frame(&frame, 1000);
    assert!(!first.should_skip && first.is_keepalive, "首帧必须保活放行");
    assert_eq!(
        gate.bypassed_frames(),
        0,
        "DMA-BUF 帧必须真正进入门控判定，不允许落入绕过分支"
    );

    // 2. 静止画面：第二帧必须被门控跳过（修复前此处恒为放行，NPU 空转）
    let second = gate.evaluate_frame(&frame, 1100);
    assert!(
        second.should_skip,
        "静态画面第二帧必须跳过推理，实际 motion_score={}",
        second.motion_score
    );

    // 3. 真实运动：写入高亮矩形后必须立即放行
    paint_motion_block(&source_fd, 250);
    let third = gate.evaluate_frame(&frame, 1200);
    assert!(
        !third.should_skip && !third.is_keepalive,
        "检测到运动必须放行推理，实际 motion_score={}",
        third.motion_score
    );

    // 4. 运动过后再次静止：参考帧已更新为新场景，必须重新进入跳过态
    let fourth = gate.evaluate_frame(&frame, 1300);
    assert!(
        fourth.should_skip,
        "场景稳定后必须恢复跳过推理，实际 motion_score={}",
        fourth.motion_score
    );
}
