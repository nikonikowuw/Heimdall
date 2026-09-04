# Rust ↔ C++ 边界规范

> 三个平台 SDK 都是 C/C++ 接口。**`unsafe` 必须收敛在极薄的一层里**，出了这层就是安全 Rust。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 绑定方式（cxx / bindgen / 手写）尚未选定，见文末待验证事项。首批绑定落地后需回填真实示例并删除本提示。

---

## 三层结构

Rust 与底层专有硬件 SDK 的交互统一采用三层封装（优先借力开源 sys-crate）：

```
crates/infer/src/backends/rknn.rs   ← ③ 安全层：实现 InferenceBackend trait
crates/media/src/decoders/mpp.rs    ← ③ 安全层：实现 VideoDecoder trait
              │
              ▼
crates/infer/src/backends/rknn/ffi.rs ← ② unsafe 绑定层：唯一允许 unsafe 与裸指针的地方
(或借力生态绑定如 objc2)
              │
              ▼
native/rknn/include/argus_rknn.h    ← ① 极薄平台 C 接口：唯一的跨语言直接调用契约
(仅几百行代码，封装底层 DMA-BUF / 硬件驱动)
```

规则：

- **生态优先与极薄垫片结合**：通用跨平台推理优先采用成熟开源库（如 `ort`、`coreml-rs`）；针对专有异构芯片驱动（Rockchip MPP/RGA/RKNN、Ascend DVPP/ACL），统一经由 `native/` 微型 C 垫片封装标准 C ABI，在 Rust 侧安全接入。
- **`unsafe` 只允许收敛在各 backend 的 `ffi.rs` 或 sys-crate 交互处**。业务层 `pipeline` / `api` 严禁出现任何 `unsafe`。
- **安全层不出现裸指针、`c_int`、`c_void`**。它的签名只面向 Rust 原生类型、`Result` 与领域结构。
- 平台胶合代码控制在百行量级，只做硬件上下文初始化和参数转发，不包含任何业务与调度逻辑。

---

## C ABI 设计约定

C++ 内部随便用 C++ 特性，但**跨界面只允许**：

| 允许 | 不允许 |
|------|--------|
| `extern "C"` 符号与函数指针表 | C++ 名称修饰（name mangling）的函数 |
| 不透明指针（`typedef void* av_algo_library`、`av_algo_instance`） | 暴露 C++ 类成员内存布局 |
| POD / 固定对齐结构体（`#pragma pack(push, 8)`） | `std::string`、`std::vector` 跨界 |
| `int32_t` 状态码返回（`av_algo_status` 枚举） | C++ 异常跨界（**会直接 UB**） |
| 函数指针虚表（`av_algo_abi` 结构体） | C++ 虚基类（vtable 实现跨编译器不兼容） |

```c
/* sdk/include/argus/types.h 核心契约节选 */
#pragma pack(push, 8)
typedef struct av_frame_desc {
    uint32_t size;                 /* 0: 必须 == 152 */
    uint32_t api_version;          /* 4: AV_ALGO_API_VERSION */
    uint64_t frame_id;             /* 8 */
    int64_t  wall_time_ns;         /* 16: 物理墙上时间 (纳秒) */
    int64_t  pts_ns;               /* 24: 视频呈现时间 (纳秒) */
    uint64_t modifier;             /* 32: DRM modifier */
    uint64_t offset[4];            /* 40: Y/U/V 内存平面偏移 */
    void*    opaque;               /* 72: 平台原生句柄 (DMA-BUF fd / CVPixelBufferRef) */
    void*    frame_token;          /* 80: 帧生命周期引用句柄 */
    uint32_t platform_tag;         /* 88 */
    uint32_t opaque_kind;          /* 92: AV_OPAQUE_DMABUF / AV_OPAQUE_CVPIXELBUFFER */
    uint32_t memory_type;          /* 96 */
    uint32_t pixel_format;         /* 100: AV_PIX_NV12 等 */
    uint32_t layout;               /* 104 */
    uint32_t width;                /* 108: 有效像素宽度 */
    uint32_t height;               /* 112: 有效像素高度 */
    uint32_t alloc_width;          /* 116: 硬件分配/步长虚宽 (hor_stride) */
    uint32_t alloc_height;         /* 120: 硬件分配/步长虚高 (ver_stride) */
    int32_t  stride[4];            /* 124: 各平面字节步长 */
    uint16_t color_primaries;      /* 140 */
    /* ... 补齐至 152 字节 */
} av_frame_desc;
#pragma pack(pop)
```

## 极薄平台硬件垫片 C 接口示例

```c
/* native/rknn/include/argus_rknn.h */
#ifdef __cplusplus
extern "C" {
#endif

typedef struct ArgusRknnSession ArgusRknnSession;

/* 会话生命周期与硬件直通调用 */
int32_t argus_rknn_create(const char* model_path, ArgusRknnSession** out);
int32_t argus_rknn_infer_dmabuf(ArgusRknnSession* s, int32_t fd,
                                uint32_t width, uint32_t height,
                                float* out_boxes, uint32_t out_cap, uint32_t* out_len);
void    argus_rknn_destroy(ArgusRknnSession* s);

#ifdef __cplusplus
}
#endif
```

要点：

- **C++ 异常绝不能穿过 `extern "C"` 边界**。每个导出函数体外层必须包 `try { ... } catch (...) { return ERR; }`，建议修饰 `noexcept`。C++ 异常跨过 FFI 边界在 Rust 侧是立即 UB。
- **Rust 反向回调严禁 panic 逃逸**：Rust 侧回调入口必须使用 `std::panic::catch_unwind` 捕获所有 panic，严禁向 C++ 堆栈 unwind。
- **强制内存布局断言**：C 侧使用 `AV_STATIC_ASSERT` 校验 `sizeof` 和关键 `offsetof`，Rust 侧单元测试必须增加对应的 `std::mem::size_of` 与 `std::mem::offset_of!` 断言，保证双向零偏差。
- **创建/销毁成对出现**，Rust 侧用 `Drop` 保证调用 destroy。

---

## unsafe 绑定层写法

```rust
// crates/infer/src/backends/rknn/ffi.rs
use std::ffi::CString;

pub(super) struct Session {
    raw: *mut ArgusRknnSession,
}

// SAFETY: RKNN session 内部无跨线程共享状态，但同一 session 不可并发调用，
// 因此只实现 Send 不实现 Sync；调用方保证一个 session 绑一个线程。
unsafe impl Send for Session {}

impl Session {
    pub(super) fn create(model_path: &Path) -> Result<Self, BackendError> {
        let c_path = CString::new(model_path.as_os_str().as_bytes())
            .map_err(|_| BackendError::InvalidPath)?;
        let mut raw = std::ptr::null_mut();
        // SAFETY: c_path 在调用期间存活；out 指向有效的栈变量。
        let code = unsafe { argus_rknn_create(c_path.as_ptr(), &mut raw) };
        check(code, "rknn_create")?;
        debug_assert!(!raw.is_null());
        Ok(Self { raw })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: raw 由 create 产出且未被释放过；Drop 只执行一次。
        unsafe { argus_rknn_destroy(self.raw) };
    }
}
```

强制约定：

- **每个 `unsafe` 块上方必须有 `// SAFETY:` 注释**，说明为什么这里的前置条件成立。没有 SAFETY 注释的 `unsafe` 不允许合并。clippy 的 `undocumented_unsafe_blocks` 要开。
- **每个 `unsafe impl Send/Sync` 必须有 SAFETY 注释**，说明依据。这是最容易出错的地方 —— 很多 SDK 的 context 实际不能跨线程。
- **裸指针不逃逸出 `ffi.rs`**。
- **`CString` 的生命周期要覆盖调用**：`CString::new(s).unwrap().as_ptr()` 是经典悬垂指针 bug（临时值当场析构），必须先绑定到变量。

---

## 构建集成（极薄硬件垫片）

在平台 backend crate（如 `crates/infer/build.rs`）中通过 `cc` crate 直接编译 C 胶合代码并链接平台 SDK：

```rust
// crates/infer/build.rs
fn main() {
    #[cfg(feature = "backend-rknn")]
    {
        cc::Build::new()
            .file("../../native/rknn/src/session.c")
            .include("../../native/rknn/include")
            .compile("argus_rknn_shim");
        println!("cargo:rustc-link-lib=dylib=rknnrt");
        println!("cargo:rerun-if-changed=../../native/rknn");
    }
}
```

约定：

- **极简构建**：无复杂 CMake 依赖，几十行 C 胶合代码直接通过 `cc` 编译并打入静态库。
- **必须写 `cargo:rerun-if-changed`**，改动 C 代码时自动触发重编。

---

## 头文件与 Rust 声明的同步

C 声明在两边各写一份，改一边忘另一边就是 UB。缓解手段（选型见待验证事项）：

- 用 `bindgen` 从头文件自动生成 Rust 声明 → 天然同步，但引入 clang 构建依赖
- 用 `cxx` → 双向类型检查最强，但要求 C++ 侧按 cxx 的规矩写
- 手写 → 无额外依赖，但必须有测试兜底

**无论选哪种，都要有一个 `size_of` / `align_of` 的断言测试**，验证跨界 POD 结构体在两侧布局一致。

---

## 禁止事项

- ❌ `unsafe` 出现在 `ffi.rs` 之外
- ❌ 没有 `// SAFETY:` 注释的 `unsafe` 块
- ❌ C++ 异常穿过 `extern "C"` 边界
- ❌ `CString::new(x).unwrap().as_ptr()` 这类临时值取指针
- ❌ 裸指针出现在安全层的函数签名里
- ❌ 跨界传 `std::string` / `std::vector`
- ❌ 硬编码 SDK 绝对路径
- ❌ 漏写 `cargo:rerun-if-changed`

---

## 待验证事项

- [ ] `bindgen` 生成的 Rust bindings 是否作为 pre-generated 文件入 git 还是由 `build.rs` 实时生成
- [ ] 动态加载模式下，`libloading` 加载 `av_algo_get_abi` 符号时的安全抽象封装
- [ ] C++ 侧动态插件在 macOS 与 Linux 上的符号导出宏（`AV_ALGO_EXPORT`）与运行时兼容性
