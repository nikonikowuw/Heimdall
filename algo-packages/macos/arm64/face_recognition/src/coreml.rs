//! macOS CoreML/ANE 双模型执行器。
//!
//! 所有 Objective-C、CoreVideo 裸指针和 `unsafe impl` 都限制在本文件内；上层只接触
//! `CoreMlFaceModels` 的 `Vec<f32>` 结果和 RAII 像素缓冲区。

#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr::{self, null_mut};

use algo_sdk::error::AlgoError;

#[allow(clippy::duplicated_attributes)]
#[link(name = "objc", kind = "dylib")]
#[link(name = "Foundation", kind = "framework")]
#[link(name = "CoreML", kind = "framework")]
#[link(name = "CoreVideo", kind = "framework")]
#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_retain(obj: *mut c_void) -> *mut c_void;
    fn objc_release(obj: *mut c_void);
    fn vImageConvert_Planar16FtoPlanarF(
        src: *const VImageBuffer,
        dest: *const VImageBuffer,
        flags: u32,
    ) -> isize;

    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format_type: u32,
        pixel_buffer_attributes: *const c_void,
        pixel_buffer_out: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut c_void) -> usize;
}

const K_CVPIXEL_FORMAT_32_BGRA: u32 = 0x4247_5241;
const ML_FEATURE_TYPE_IMAGE: isize = 4;
const ML_FEATURE_TYPE_MULTI_ARRAY: isize = 5;
const ML_DATA_TYPE_FLOAT16: isize = 65_552;
const ML_DATA_TYPE_FLOAT32: isize = 65_568;
const ML_DATA_TYPE_DOUBLE: isize = 65_600;
const MAX_OUTPUT_ELEMENTS: usize = 4 * 1024 * 1024;

#[repr(C)]
struct VImageBuffer {
    data: *mut c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

struct AutoreleasePool(*mut c_void);

impl AutoreleasePool {
    fn new() -> Self {
        // SAFETY: 系统调用创建一个当前线程拥有的 Objective-C 自动释放池。
        let pool = unsafe { objc_autoreleasePoolPush() };
        Self(pool)
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: pool 由 objc_autoreleasePoolPush 返回，且只在 Drop 中弹出一次。
            unsafe { objc_autoreleasePoolPop(self.0) };
        }
    }
}

macro_rules! objc_send {
    ($target:expr, $selector:expr $(, $arg:expr)*) => {{
        #[allow(unused_unsafe)]
        {
            let function: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void $(, objc_send!(@type $arg))*
            ) -> *mut c_void =
                // SAFETY: objc_msgSend 的地址按当前 selector 的确切 C 函数签名转换。
                unsafe { std::mem::transmute(objc_msgSend as *const ()) };
            // SAFETY: 调用方保证 target、selector 和参数符合 Objective-C 方法签名。
            unsafe { function($target, $selector $(, $arg)*) }
        }
    }};
    (@type $arg:expr) => { _ };
}

macro_rules! objc_send_isize {
    ($target:expr, $selector:expr $(, $arg:expr)*) => {{
        #[allow(unused_unsafe)]
        {
            let function: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void $(, objc_send_isize!(@type $arg))*
            ) -> isize =
                // SAFETY: objc_msgSend 的地址按当前 selector 的确切整数返回签名转换。
                unsafe { std::mem::transmute(objc_msgSend as *const ()) };
            // SAFETY: 调用方保证 target、selector 和参数符合 Objective-C 方法签名。
            unsafe { function($target, $selector $(, $arg)*) }
        }
    }};
    (@type $arg:expr) => { _ };
}

macro_rules! objc_send_void {
    ($target:expr, $selector:expr $(, $arg:expr)*) => {{
        #[allow(unused_unsafe)]
        {
            let function: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void $(, objc_send_void!(@type $arg))*
            ) =
                // SAFETY: objc_msgSend 的地址按当前 selector 的确切 void 返回签名转换。
                unsafe { std::mem::transmute(objc_msgSend as *const ()) };
            // SAFETY: 调用方保证 target、selector 和参数符合 Objective-C 方法签名。
            unsafe { function($target, $selector $(, $arg)*) }
        }
    }};
    (@type $arg:expr) => { _ };
}

fn get_class(name: &str) -> Result<*mut c_void, AlgoError> {
    let name = CString::new(name).map_err(|error| AlgoError::Internal {
        reason: format!("Objective-C 类名非法: {error}"),
    })?;
    // SAFETY: name 是有效的 NUL 结尾 C 字符串。
    let class = unsafe { objc_getClass(name.as_ptr()) };
    if class.is_null() {
        Err(AlgoError::Internal {
            reason: "Objective-C 类不存在".to_string(),
        })
    } else {
        Ok(class)
    }
}

fn register_selector(name: &str) -> Result<*mut c_void, AlgoError> {
    let name = CString::new(name).map_err(|error| AlgoError::Internal {
        reason: format!("Objective-C 选择器名非法: {error}"),
    })?;
    // SAFETY: name 是有效的 NUL 结尾 C 字符串；运行时会驻留 selector。
    let selector = unsafe { sel_registerName(name.as_ptr()) };
    if selector.is_null() {
        Err(AlgoError::Internal {
            reason: "Objective-C 选择器注册失败".to_string(),
        })
    } else {
        Ok(selector)
    }
}

fn ns_string(value: &str) -> Result<*mut c_void, AlgoError> {
    let value = CString::new(value).map_err(|error| AlgoError::Internal {
        reason: format!("NSString 内容非法: {error}"),
    })?;
    let class = get_class("NSString")?;
    let selector = register_selector("stringWithUTF8String:")?;
    let string = objc_send!(class, selector, value.as_ptr());
    if string.is_null() {
        Err(AlgoError::Internal {
            reason: "创建 NSString 失败".to_string(),
        })
    } else {
        Ok(string)
    }
}

fn ns_error_description(error: *mut c_void) -> String {
    if error.is_null() {
        return "未知 CoreML 错误".to_string();
    }
    let Ok(description_selector) = register_selector("localizedDescription") else {
        return "NSError (无法获取描述)".to_string();
    };
    let description = objc_send!(error, description_selector);
    if description.is_null() {
        return "NSError (描述为空)".to_string();
    }
    let Ok(utf8_selector) = register_selector("UTF8String") else {
        return "NSError (无法获取 UTF8 描述)".to_string();
    };
    let function: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const c_char =
        // SAFETY: objc_msgSend 被绑定为 NSString::UTF8String 的正确返回类型。
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    // SAFETY: description 是有效 NSString，selector 返回其内部 NUL 结尾 UTF-8 指针。
    let utf8 = unsafe { function(description, utf8_selector) };
    if utf8.is_null() {
        "NSError (UTF8 描述为空)".to_string()
    } else {
        // SAFETY: NSString::UTF8String 返回的指针在 description 存活期间有效。
        unsafe { CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

struct PixelBufferLock {
    ptr: *mut c_void,
    flags: u64,
}

impl PixelBufferLock {
    fn new(ptr: *mut c_void, flags: u64) -> Result<Self, AlgoError> {
        if ptr.is_null() {
            return Err(AlgoError::Preprocess {
                reason: "CVPixelBuffer 指针为空".to_string(),
            });
        }
        // SAFETY: ptr 由 CoreVideo 创建或由 ABI 借用，且当前调用持有其所有权/生命周期。
        let status = unsafe { CVPixelBufferLockBaseAddress(ptr, flags) };
        if status != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("锁定 CVPixelBuffer 失败: {status}"),
            });
        }
        Ok(Self { ptr, flags })
    }
}

impl Drop for PixelBufferLock {
    fn drop(&mut self) {
        // SAFETY: 当前 guard 只对成功 lock 的同一 buffer 执行一次 unlock。
        unsafe {
            CVPixelBufferUnlockBaseAddress(self.ptr, self.flags);
        }
    }
}

/// 由本模块创建并独占释放的 BGRA CVPixelBuffer。
#[derive(Debug)]
pub struct OwnedPixelBuffer(*mut c_void);

impl OwnedPixelBuffer {
    pub fn from_rgb(rgb: &[u8], width: u32, height: u32) -> Result<Self, AlgoError> {
        let width_usize = width as usize;
        let height_usize = height as usize;
        let expected = width_usize
            .checked_mul(height_usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;
        if width == 0 || height == 0 || rgb.len() < expected {
            return Err(AlgoError::Preprocess {
                reason: "RGB 输入尺寸或数据长度无效".to_string(),
            });
        }
        let mut raw = null_mut();
        // SAFETY: 输出指针指向当前栈变量；attributes 为空表示使用 CoreVideo 默认属性。
        let status = unsafe {
            CVPixelBufferCreate(
                ptr::null(),
                width_usize,
                height_usize,
                K_CVPIXEL_FORMAT_32_BGRA,
                ptr::null(),
                &mut raw,
            )
        };
        if status != 0 || raw.is_null() {
            return Err(AlgoError::Preprocess {
                reason: format!("创建 BGRA CVPixelBuffer 失败: {status}"),
            });
        }
        let buffer = Self(raw);
        let _lock = PixelBufferLock::new(raw, 0)?;
        // SAFETY: buffer 已成功 lock，CoreVideo 返回的 base address 覆盖整个 surface。
        let base = unsafe { CVPixelBufferGetBaseAddress(raw) } as *mut u8;
        // SAFETY: buffer 已成功 lock，row bytes 由 CoreVideo 返回且不小于一行像素。
        let row_bytes = unsafe { CVPixelBufferGetBytesPerRow(raw) };
        let row_len = width_usize.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
        if base.is_null() || row_bytes < row_len {
            return Err(AlgoError::Preprocess {
                reason: "BGRA CVPixelBuffer 地址或 stride 无效".to_string(),
            });
        }
        for y in 0..height_usize {
            // SAFETY: y * row_bytes 位于锁定 surface 内，row_len <= row_bytes。
            let dst = unsafe { std::slice::from_raw_parts_mut(base.add(y * row_bytes), row_len) };
            let src = &rgb[y * width_usize * 3..(y + 1) * width_usize * 3];
            for (src_pixel, dst_pixel) in src.chunks_exact(3).zip(dst.chunks_exact_mut(4)) {
                dst_pixel[0] = src_pixel[2];
                dst_pixel[1] = src_pixel[1];
                dst_pixel[2] = src_pixel[0];
                dst_pixel[3] = 255;
            }
        }
        Ok(buffer)
    }

    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for OwnedPixelBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 由 CVPixelBufferCreate 返回且只由当前 RAII 值释放一次。
            unsafe { CVPixelBufferRelease(self.0) };
        }
    }
}

#[derive(Clone, Copy)]
struct CachedSelectors {
    feature_value_class: *mut c_void,
    feature_value_pixel_buffer: *mut c_void,
    dictionary_class: *mut c_void,
    dictionary_with_object: *mut c_void,
    provider_class: *mut c_void,
    alloc: *mut c_void,
    init_dictionary: *mut c_void,
    prediction: *mut c_void,
    feature_for_name: *mut c_void,
    multi_array_value: *mut c_void,
    count: *mut c_void,
    data_pointer: *mut c_void,
    shape: *mut c_void,
    strides: *mut c_void,
    object_at_index: *mut c_void,
    integer_value: *mut c_void,
    data_type: *mut c_void,
}

/// 一个已加载的 CoreML 模型，模型和 selector 均在加载阶段缓存。
pub struct CoreMlRunner {
    model: *mut c_void,
    input_name: *mut c_void,
    output_name: *mut c_void,
    selectors: CachedSelectors,
}

impl std::fmt::Debug for CoreMlRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoreMlRunner")
            .field("model_loaded", &(!self.model.is_null()))
            .finish()
    }
}

// SAFETY: MLModel prediction 使用只读模型上下文；模型对象和 selector 在本结构体存活期间
// 均保持有效引用，CoreML 文档保证 prediction 可从多个线程调用。
unsafe impl Send for CoreMlRunner {}
// SAFETY: 同上；所有可变的临时 provider 和输出对象均由单次 prediction 调用独占。
unsafe impl Sync for CoreMlRunner {}

impl CoreMlRunner {
    pub fn load_model(
        package_root: &Path,
        file_name: &str,
        fallback_input: &str,
        fallback_output: &str,
    ) -> Result<Self, AlgoError> {
        let model_path = resolve_model_path(package_root, file_name)?;
        let path_string = model_path.to_str().ok_or_else(|| AlgoError::ModelLoad {
            reason: "CoreML 模型路径不是合法 UTF-8".to_string(),
        })?;
        let _pool = AutoreleasePool::new();
        let path_string_ns = ns_string(path_string)?;
        let nsurl_class = get_class("NSURL")?;
        let file_url = register_selector("fileURLWithPath:")?;
        let model_url = objc_send!(nsurl_class, file_url, path_string_ns);
        if model_url.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: "创建 CoreML 模型 NSURL 失败".to_string(),
            });
        }

        let mlmodel_class = get_class("MLModel")?;
        let compiled_url = if matches!(
            model_path.extension().and_then(|ext| ext.to_str()),
            Some("mlpackage" | "mlmodel")
        ) {
            let compile_selector = register_selector("compileModelAtURL:error:")?;
            let mut error: *mut c_void = null_mut();
            let compiled = objc_send!(mlmodel_class, compile_selector, model_url, &mut error);
            if compiled.is_null() || !error.is_null() {
                return Err(AlgoError::ModelLoad {
                    reason: format!("编译 CoreML 模型失败: {}", ns_error_description(error)),
                });
            }
            compiled
        } else {
            model_url
        };

        let configuration_class = get_class("MLModelConfiguration")?;
        let alloc_selector = register_selector("alloc")?;
        let init_selector = register_selector("init")?;
        let configuration = objc_send!(
            objc_send!(configuration_class, alloc_selector),
            init_selector
        );
        if configuration.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: "创建 MLModelConfiguration 失败".to_string(),
            });
        }
        let compute_units = register_selector("setComputeUnits:")?;
        // SAFETY: 2 是公开枚举 MLComputeUnitsAll，启用 CPU/GPU/ANE 自动调度。
        objc_send_void!(configuration, compute_units, 2isize);

        let load_selector = register_selector("modelWithContentsOfURL:configuration:error:")?;
        let mut error: *mut c_void = null_mut();
        let model = objc_send!(
            mlmodel_class,
            load_selector,
            compiled_url,
            configuration,
            &mut error
        );
        if model.is_null() || !error.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: format!("加载 CoreML 模型失败: {}", ns_error_description(error)),
            });
        }
        // SAFETY: model 由 CoreML 返回；retain 后所有权由 CoreMlRunner::drop 配对释放。
        let model = unsafe { objc_retain(model) };

        let model_description = register_selector("modelDescription")?;
        let input_descriptions = register_selector("inputDescriptionsByName")?;
        let output_descriptions = register_selector("outputDescriptionsByName")?;
        let all_keys = register_selector("allKeys")?;
        let object_for_key = register_selector("objectForKey:")?;
        let feature_type = register_selector("type")?;
        let count = register_selector("count")?;
        let object_at_index = register_selector("objectAtIndex:")?;
        let mut input_name = null_mut();
        let mut output_name = null_mut();
        let description = objc_send!(model, model_description);
        if !description.is_null() {
            let inputs = objc_send!(description, input_descriptions);
            let input_keys = if inputs.is_null() {
                null_mut()
            } else {
                objc_send!(inputs, all_keys)
            };
            if !input_keys.is_null() {
                let input_count = objc_send_isize!(input_keys, count);
                for index in 0..input_count {
                    let key = objc_send!(input_keys, object_at_index, index);
                    let feature = objc_send!(inputs, object_for_key, key);
                    if objc_send_isize!(feature, feature_type) == ML_FEATURE_TYPE_IMAGE {
                        input_name = key;
                        break;
                    }
                }
            }

            let outputs = objc_send!(description, output_descriptions);
            let output_keys = if outputs.is_null() {
                null_mut()
            } else {
                objc_send!(outputs, all_keys)
            };
            if !output_keys.is_null() {
                let output_count = objc_send_isize!(output_keys, count);
                for index in 0..output_count {
                    let key = objc_send!(output_keys, object_at_index, index);
                    let feature = objc_send!(outputs, object_for_key, key);
                    if objc_send_isize!(feature, feature_type) == ML_FEATURE_TYPE_MULTI_ARRAY {
                        output_name = key;
                        break;
                    }
                }
            }
        }
        if input_name.is_null() {
            input_name = ns_string(fallback_input)?;
        }
        if output_name.is_null() {
            output_name = ns_string(fallback_output)?;
        }
        // SAFETY: 模型描述或 ns_string 返回的对象在当前 pool 中有效；runner 需要跨 pool 持有。
        let input_name = unsafe { objc_retain(input_name) };
        // SAFETY: 同上。
        let output_name = unsafe { objc_retain(output_name) };
        if model.is_null() || input_name.is_null() || output_name.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: "CoreML 模型对象引用为空".to_string(),
            });
        }

        let selectors = CachedSelectors {
            feature_value_class: get_class("MLFeatureValue")?,
            feature_value_pixel_buffer: register_selector("featureValueWithPixelBuffer:")?,
            dictionary_class: get_class("NSDictionary")?,
            dictionary_with_object: register_selector("dictionaryWithObject:forKey:")?,
            provider_class: get_class("MLDictionaryFeatureProvider")?,
            alloc: alloc_selector,
            init_dictionary: register_selector("initWithDictionary:error:")?,
            prediction: register_selector("predictionFromFeatures:error:")?,
            feature_for_name: register_selector("featureValueForName:")?,
            multi_array_value: register_selector("multiArrayValue")?,
            count: register_selector("count")?,
            data_pointer: register_selector("dataPointer")?,
            shape: register_selector("shape")?,
            strides: register_selector("strides")?,
            object_at_index,
            integer_value: register_selector("integerValue")?,
            data_type: register_selector("dataType")?,
        };

        Ok(Self {
            model,
            input_name,
            output_name,
            selectors,
        })
    }

    /// 在给定 CVPixelBuffer 上执行模型推理并按逻辑顺序返回浮点张量。
    ///
    /// # Safety
    /// `pixel_buffer` 必须是当前调用期间保持有效的 CoreVideo pixel buffer。
    pub unsafe fn predict_pixelbuffer(
        &self,
        pixel_buffer: *mut c_void,
    ) -> Result<Vec<f32>, AlgoError> {
        if pixel_buffer.is_null() {
            return Err(AlgoError::Inference {
                reason: "CoreML 输入 CVPixelBuffer 为空".to_string(),
            });
        }
        let _pool = AutoreleasePool::new();
        let selectors = self.selectors;
        let feature_value = objc_send!(
            selectors.feature_value_class,
            selectors.feature_value_pixel_buffer,
            pixel_buffer
        );
        if feature_value.is_null() {
            return Err(AlgoError::Inference {
                reason: "创建 MLFeatureValue 失败".to_string(),
            });
        }
        let dictionary = objc_send!(
            selectors.dictionary_class,
            selectors.dictionary_with_object,
            feature_value,
            self.input_name
        );
        if dictionary.is_null() {
            return Err(AlgoError::Inference {
                reason: "创建 CoreML 输入字典失败".to_string(),
            });
        }
        let mut error: *mut c_void = null_mut();
        let provider = objc_send!(
            objc_send!(selectors.provider_class, selectors.alloc),
            selectors.init_dictionary,
            dictionary,
            &mut error
        );
        if provider.is_null() || !error.is_null() {
            return Err(AlgoError::Inference {
                reason: format!(
                    "创建 MLFeatureProvider 失败: {}",
                    ns_error_description(error)
                ),
            });
        }
        error = null_mut();
        let output_provider = objc_send!(self.model, selectors.prediction, provider, &mut error);
        if output_provider.is_null() || !error.is_null() {
            return Err(AlgoError::Inference {
                reason: format!("CoreML 前向推理失败: {}", ns_error_description(error)),
            });
        }
        let output_feature = objc_send!(
            output_provider,
            selectors.feature_for_name,
            self.output_name
        );
        if output_feature.is_null() {
            return Err(AlgoError::Inference {
                reason: "CoreML 输出特征不存在".to_string(),
            });
        }
        let array = objc_send!(output_feature, selectors.multi_array_value);
        if array.is_null() {
            return Err(AlgoError::Inference {
                reason: "CoreML 输出不是 MLMultiArray".to_string(),
            });
        }
        let count = objc_send_isize!(array, selectors.count);
        if count <= 0 || count as usize > MAX_OUTPUT_ELEMENTS {
            return Err(AlgoError::Inference {
                reason: format!("CoreML 输出元素数非法: {count}"),
            });
        }
        let data_pointer = objc_send!(array, selectors.data_pointer);
        if data_pointer.is_null() {
            return Err(AlgoError::Inference {
                reason: "CoreML 输出 dataPointer 为空".to_string(),
            });
        }
        let shape = read_ns_integer_array(objc_send!(array, selectors.shape), selectors);
        let strides = read_ns_integer_array(objc_send!(array, selectors.strides), selectors);
        let offsets = logical_offsets(count as usize, &shape, &strides)?;
        let data_type = objc_send_isize!(array, selectors.data_type);
        read_tensor(data_pointer, data_type, &offsets)
    }

    pub fn predict_rgb(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>, AlgoError> {
        let pixel_buffer = OwnedPixelBuffer::from_rgb(rgb, width, height)?;
        // SAFETY: pixel_buffer 在推理完成前保持所有权和有效生命周期。
        unsafe { self.predict_pixelbuffer(pixel_buffer.as_ptr()) }
    }
}

impl Drop for CoreMlRunner {
    fn drop(&mut self) {
        // SAFETY: 三个对象均由本结构体 retain，且 Drop 只执行一次。
        unsafe {
            if !self.output_name.is_null() {
                objc_release(self.output_name);
            }
            if !self.input_name.is_null() {
                objc_release(self.input_name);
            }
            if !self.model.is_null() {
                objc_release(self.model);
            }
        }
    }
}

fn resolve_model_path(package_root: &Path, file_name: &str) -> Result<PathBuf, AlgoError> {
    let path = package_root.join("model").join(file_name);
    if path.exists() {
        return Ok(path);
    }
    Err(AlgoError::ModelLoad {
        reason: format!("模型文件不存在: {}", path.display()),
    })
}

fn read_ns_integer_array(array: *mut c_void, selectors: CachedSelectors) -> Vec<isize> {
    if array.is_null() {
        return Vec::new();
    }
    let count = objc_send_isize!(array, selectors.count);
    if count <= 0 || count > 16 {
        return Vec::new();
    }
    let mut values = Vec::with_capacity(count as usize);
    for index in 0..count {
        let object = objc_send!(array, selectors.object_at_index, index);
        if object.is_null() {
            return Vec::new();
        }
        values.push(objc_send_isize!(object, selectors.integer_value));
    }
    values
}

fn logical_offsets(
    count: usize,
    shape: &[isize],
    strides: &[isize],
) -> Result<Vec<usize>, AlgoError> {
    if shape.is_empty() || shape.len() != strides.len() {
        return Ok((0..count).collect());
    }
    if shape
        .iter()
        .any(|dimension| *dimension <= 0 || *dimension as usize > MAX_OUTPUT_ELEMENTS)
        || strides.iter().any(|stride| *stride < 0)
    {
        return Err(AlgoError::Inference {
            reason: "CoreML 输出 shape/stride 非法".to_string(),
        });
    }
    let shape_product = shape.iter().try_fold(1usize, |product, dimension| {
        product.checked_mul(*dimension as usize)
    });
    if shape_product != Some(count) {
        return Err(AlgoError::Inference {
            reason: "CoreML 输出 shape 与 count 不一致".to_string(),
        });
    }
    let mut offsets = Vec::with_capacity(count);
    for linear in 0..count {
        let mut remainder = linear;
        let mut offset = 0usize;
        for dimension in (0..shape.len()).rev() {
            let size = shape[dimension] as usize;
            let index = remainder % size;
            remainder /= size;
            offset = offset
                .checked_add(
                    index
                        .checked_mul(strides[dimension] as usize)
                        .ok_or(AlgoError::OutOfMemory)?,
                )
                .ok_or(AlgoError::OutOfMemory)?;
        }
        offsets.push(offset);
    }
    Ok(offsets)
}

fn read_tensor(
    data_pointer: *mut c_void,
    data_type: isize,
    offsets: &[usize],
) -> Result<Vec<f32>, AlgoError> {
    let mut output = vec![0.0f32; offsets.len()];
    match data_type {
        ML_DATA_TYPE_FLOAT16 => {
            let mut packed = Vec::with_capacity(offsets.len());
            for offset in offsets {
                // SAFETY: CoreML MLMultiArray dataPointer 在 count/shape/stride 校验后可读；
                // read_unaligned 允许底层分配不满足 u16 对齐。
                let value =
                    unsafe { ptr::read_unaligned((data_pointer as *const u16).add(*offset)) };
                packed.push(value);
            }
            let source = VImageBuffer {
                data: packed.as_mut_ptr().cast::<c_void>(),
                height: 1,
                width: packed.len(),
                row_bytes: packed.len() * 2,
            };
            let destination = VImageBuffer {
                data: output.as_mut_ptr().cast::<c_void>(),
                height: 1,
                width: output.len(),
                row_bytes: output.len() * 4,
            };
            // SAFETY: source/destination 覆盖相同数量的合法 f16/f32 元素。
            let status = unsafe { vImageConvert_Planar16FtoPlanarF(&source, &destination, 0) };
            if status != 0 {
                return Err(AlgoError::Inference {
                    reason: format!("Float16 转 Float32 失败: {status}"),
                });
            }
        }
        ML_DATA_TYPE_FLOAT32 => {
            for (index, offset) in offsets.iter().enumerate() {
                // SAFETY: CoreML 输出数据由模型 provider 持有且在当前 autorelease pool 内有效。
                output[index] =
                    unsafe { ptr::read_unaligned((data_pointer as *const f32).add(*offset)) };
            }
        }
        ML_DATA_TYPE_DOUBLE => {
            for (index, offset) in offsets.iter().enumerate() {
                // SAFETY: 同上，按 MLMultiArray 的 double 元素宽度读取。
                output[index] = unsafe {
                    ptr::read_unaligned((data_pointer as *const f64).add(*offset)) as f32
                };
            }
        }
        other => {
            return Err(AlgoError::Inference {
                reason: format!("不支持的 CoreML 输出数据类型: {other}"),
            });
        }
    }
    if output.iter().any(|value| !value.is_finite()) {
        return Err(AlgoError::Inference {
            reason: "CoreML 输出包含 NaN 或无穷值".to_string(),
        });
    }
    Ok(output)
}

/// 人脸检测和 EdgeFace 特征提取的双模型持有者。
#[derive(Debug)]
pub struct CoreMlFaceModels {
    pub detector: CoreMlRunner,
    pub embedder: CoreMlRunner,
}

impl CoreMlFaceModels {
    pub fn load(package_root: &Path) -> Result<Self, AlgoError> {
        let detector_model_name = if package_root.join("model/yolov8_face.mlpackage").exists() {
            "yolov8_face.mlpackage"
        } else {
            "yolov5n_face.mlpackage"
        };
        Ok(Self {
            detector: CoreMlRunner::load_model(
                package_root,
                detector_model_name,
                "image",
                "var_911",
            )?,
            embedder: CoreMlRunner::load_model(
                package_root,
                "edgeface_s.mlpackage",
                "input",
                "embedding",
            )?,
        })
    }

    /// # Safety
    /// `pixel_buffer` 必须在当前调用期间保持有效。
    pub unsafe fn predict_detector(
        &self,
        pixel_buffer: *mut c_void,
    ) -> Result<Vec<f32>, AlgoError> {
        // SAFETY: 由调用方保证 pixel_buffer 生命周期，本函数仅借用它做同步预测。
        unsafe { self.detector.predict_pixelbuffer(pixel_buffer) }
    }

    pub fn predict_embedding(&self, rgb: &[u8]) -> Result<Vec<f32>, AlgoError> {
        self.embedder.predict_rgb(rgb, 112, 112)
    }
}
