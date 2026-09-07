# Rust ↔ C/C++ FFI 边界规范 (FFI Guidelines)

> 专有硬件 SDK（MPP/RGA/RKNN、DVPP/ACL、VideoToolbox）多为 C/C++ 接口。
> 核心原则：**`unsafe` 严格收敛于极薄的 FFI 边界，裸指针绝不逃逸，安全层零感知**。

---

## 1. 三层封装架构

```text
crates/infer/src/backends/rknn.rs   ← ③ 安全层：实现 InferenceBackend trait（纯安全 Rust）
              │
              ▼
crates/infer/src/backends/rknn/ffi.rs ← ② unsafe 绑定层：唯一允许 unsafe、裸指针与 Send 实现的地方
              │
              ▼
native/rknn/include/argus_rknn.h    ← ① 极薄平台 C 垫片（< 300 行）：纯 C ABI 符号导出与硬件直通
```

- **Unsafe 边界隔离铁律**：`unsafe` 只允许收敛于各 backend/media 的 `ffi.rs` 或 sys-crate 调用边界。上层业务代码（`pipeline` / `api`）**严禁出现任何 `unsafe`**；
- **安全层签名洁净度**：安全层 trait 与方法签名中**严禁出现裸指针、`c_void`、`c_int` 等 C 类型**，统一转换为 Rust 原生类型、`Result<T, E>` 与 RAII 句柄；
- **极薄垫片原则**：`native/` 下的 C 垫片仅处理底层驱动初始化与参数转发，绝不包含任何业务与调度逻辑。

---

## 2. 跨边界 C ABI 契约

C/C++ 与 Rust 跨界交互**严格受限于稳定 C ABI**：

| 允许 | 严格禁止 (Undefined Behavior) |
|------|------------------------------|
| `extern "C"` 符号与虚表函数指针 | C++ Name Mangling 修饰函数 |
| 不透明句柄（`typedef void* av_algo_instance`） | 暴露非 POD 的 C++ 类成员内存布局 |
| 固定对齐 POD 结构体（`#pragma pack(push, 8)`） | 跨界传递 `std::string`、`std::vector` |
| `int32_t` 状态码返回 | **C++ 异常跨越边界**（直接导致进程崩溃/UB） |
| `std::panic::catch_unwind` 隔离 | **Rust panic unwind 跨越边界** |

- **C++ 异常防护**：C++ 导出函数体必须用 `try { ... } catch (...) { return ERR; }` 全包裹，严禁任何异常逃逸；
- **Rust Panic 防护**：Rust 侧导出的 C ABI 回调函数入口必须用 `catch_unwind` 隔离，严禁向 C 栈 unwind；
- **双侧布局断言**：跨界 POD 结构体必须有双侧断言：C 侧静态断言 `sizeof`/`offsetof`，Rust 侧单元测试断言 `std::mem::size_of` 与 `std::mem::offset_of!`。

---

## 3. `unsafe` 绑定层写法与生命周期规范

```rust
// crates/infer/src/backends/rknn/ffi.rs
use std::ffi::CString;

pub(super) struct Session {
    raw: *mut ArgusRknnSession,
}

// SAFETY: RKNN session 内部无跨线程共享状态，但同一 session 不可并发调用，
// 因此只实现 Send 不实现 Sync；由上层架构保证一个 session 绑定一个专用线程。
unsafe impl Send for Session {}

impl Session {
    pub(super) fn create(model_path: &Path) -> Result<Self, BackendError> {
        // CString 必须绑定到具名变量，严禁 CString::new(...).unwrap().as_ptr() 造成临时值当场析构
        let c_path = CString::new(model_path.as_os_str().as_bytes())
            .map_err(|_| BackendError::InvalidPath)?;
        let mut raw = std::ptr::null_mut();
        // SAFETY: c_path 在调用期间保持存活；raw 指向有效的栈内存指针
        let code = unsafe { argus_rknn_create(c_path.as_ptr(), &mut raw) };
        check(code, "rknn_create")?;
        Ok(Self { raw })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: self.raw 由 create 产出且未被二次释放；Drop 只执行一次
            unsafe { argus_rknn_destroy(self.raw) };
        }
    }
}
```

- **强制 `// SAFETY:` 注释**：每一个 `unsafe` 块和 `unsafe impl Send/Sync` 上方必须写明前置条件与成立依据，未注释者禁止合入；
- **RAII 成对释放**：所有底层 C 句柄必须在 Rust 包装类型的 `Drop` 实现中自动销毁，杜绝资源泄漏。

---

## 4. 构建集成规范 (`build.rs`)

- 极薄 C 垫片在对应 crate 的 `build.rs` 中通过 `cc` crate 编译打入静态库；
- **必须声明 `cargo:rerun-if-changed`**，确保修改 C 垫片源码时自动触发重新编译。

---

## 5. 禁止事项 (Iron Rules)

- ❌ 在 `ffi.rs` 之外出现任何 `unsafe` 关键字
- ❌ 任何未附带 `// SAFETY:` 说明的 `unsafe` 块
- ❌ 允许 C++ 异常或 Rust panic 穿透 `extern "C"` 边界
- ❌ 写出 `CString::new(...).unwrap().as_ptr()` 导致野指针
- ❌ 在安全层的共有接口中暴露裸指针（`*const T` / `*mut T`）
- ❌ 跨界传递 C++ STL 容器（`std::string`, `std::vector`）
- ❌ 在 `build.rs` 中硬编码 SDK 的绝对物理安装路径
