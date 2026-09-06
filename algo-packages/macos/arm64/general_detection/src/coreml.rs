//! Apple CoreML 原生框架 Rust 安全绑定与零拷贝推理执行器

#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr::null_mut;

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
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_retain(obj: *mut c_void) -> *mut c_void;
    fn objc_release(obj: *mut c_void);
    fn vImageConvert_Planar16FtoPlanarF(
        src: *const VImageBuffer,
        dest: *const VImageBuffer,
        flags: u32,
    ) -> isize;
}

#[repr(C)]
struct VImageBuffer {
    data: *mut c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

// 动态通过 objc_msgSend 发送消息
extern "C" {
    fn objc_msgSend();
}

struct AutoreleasePool(*mut c_void);

impl AutoreleasePool {
    fn new() -> Self {
        // SAFETY: 系统调用 objc_autoreleasePoolPush 创建新的自动释放池
        let pool = unsafe { objc_autoreleasePoolPush() };
        Self(pool)
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 释放池成对弹出
            unsafe { objc_autoreleasePoolPop(self.0) };
        }
    }
}

macro_rules! msg_send {
    ($target:expr, $sel:expr $(, $arg:expr)*) => {{
        let msg_fn: unsafe extern "C" fn(*mut c_void, *mut c_void $(, msg_send!(@type $arg))* ) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        msg_fn($target, $sel $(, $arg)*)
    }};
    (@type $arg:expr) => { _ };
}

macro_rules! msg_send_isize {
    ($target:expr, $sel:expr $(, $arg:expr)*) => {{
        let msg_fn: unsafe extern "C" fn(*mut c_void, *mut c_void $(, msg_send_isize!(@type $arg))* ) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        msg_fn($target, $sel $(, $arg)*)
    }};
    (@type $arg:expr) => { _ };
}

macro_rules! msg_send_void {
    ($target:expr, $sel:expr $(, $arg:expr)*) => {{
        let msg_fn: unsafe extern "C" fn(*mut c_void, *mut c_void $(, msg_send_void!(@type $arg))* ) =
            std::mem::transmute(objc_msgSend as *const ());
        msg_fn($target, $sel $(, $arg)*);
    }};
    (@type $arg:expr) => { _ };
}

fn get_class(name: &str) -> Result<*mut c_void, AlgoError> {
    let c = CString::new(name).map_err(|e| AlgoError::Internal {
        reason: format!("类名无效: {e}"),
    })?;
    // SAFETY: 类名是以 null 结尾的有效 C 字符串
    let cls = unsafe { objc_getClass(c.as_ptr()) };
    if cls.is_null() {
        Err(AlgoError::Internal {
            reason: format!("Objective-C 类未找到: {name}"),
        })
    } else {
        Ok(cls)
    }
}

fn register_sel(name: &str) -> Result<*mut c_void, AlgoError> {
    let c = CString::new(name).map_err(|e| AlgoError::Internal {
        reason: format!("选择器名无效: {e}"),
    })?;
    // SAFETY: 选择器名是以 null 结尾的有效 C 字符串
    let sel = unsafe { sel_registerName(c.as_ptr()) };
    if sel.is_null() {
        Err(AlgoError::Internal {
            reason: format!("Objective-C 选择器注册失败: {name}"),
        })
    } else {
        Ok(sel)
    }
}

fn ns_string(s: &str) -> Result<*mut c_void, AlgoError> {
    let c = CString::new(s).map_err(|e| AlgoError::Internal {
        reason: format!("字符串包含非法 null: {e}"),
    })?;
    let cls = get_class("NSString")?;
    let sel = register_sel("stringWithUTF8String:")?;
    // SAFETY: cls 与 sel 均已验证有效，参数为有效 C 字符串
    let ns_str = unsafe { msg_send!(cls, sel, c.as_ptr()) };
    if ns_str.is_null() {
        Err(AlgoError::Internal {
            reason: "创建 NSString 失败".to_string(),
        })
    } else {
        Ok(ns_str)
    }
}

fn ns_error_desc(err: *mut c_void) -> String {
    if err.is_null() {
        return "未知错误".to_string();
    }
    let sel = register_sel("localizedDescription").unwrap_or(null_mut());
    if sel.is_null() {
        return "NSError (无法获取描述)".to_string();
    }
    // SAFETY: err 为非空 NSError 对象指针
    let desc = unsafe { msg_send!(err, sel) };
    if desc.is_null() {
        return "NSError (空描述)".to_string();
    }
    let utf8_sel = register_sel("UTF8String").unwrap_or(null_mut());
    // SAFETY: desc 为有效的 NSString 对象指针
    let utf8_ptr = unsafe {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const c_char =
            std::mem::transmute(objc_msgSend as *const ());
        f(desc, utf8_sel)
    };
    if utf8_ptr.is_null() {
        "NSError (无法提取 UTF8)".to_string()
    } else {
        // SAFETY: utf8_ptr 指向有效 C 字符串
        unsafe { CStr::from_ptr(utf8_ptr).to_string_lossy().to_string() }
    }
}

#[derive(Clone, Copy)]
struct CachedSelectors {
    feat_val_cls: *mut c_void,
    feat_pixel_sel: *mut c_void,
    dict_cls: *mut c_void,
    dict_sel: *mut c_void,
    provider_cls: *mut c_void,
    alloc_sel: *mut c_void,
    init_dict_sel: *mut c_void,
    predict_sel: *mut c_void,
    feat_for_name_sel: *mut c_void,
    multiarray_sel: *mut c_void,
    count_sel: *mut c_void,
    data_ptr_sel: *mut c_void,
    strides_sel: *mut c_void,
    obj_at_idx_sel: *mut c_void,
    int_val_sel: *mut c_void,
    datatype_sel: *mut c_void,
}

/// CoreML 模型执行器
pub struct CoreMlRunner {
    model: *mut c_void,
    input_name_ns: *mut c_void,
    output_name_ns: *mut c_void,
    selectors: CachedSelectors,
}

impl std::fmt::Debug for CoreMlRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreMlRunner")
            .field("model", &(!self.model.is_null()))
            .finish()
    }
}

// SAFETY: MLModel 线程安全且支持并发 prediction 调用
unsafe impl Send for CoreMlRunner {}
// SAFETY: CoreML MLModel 是只读线程安全模型上下文，支持跨线程并发共享访问
unsafe impl Sync for CoreMlRunner {}

impl CoreMlRunner {
    /// 加载模型文件（支持自动检测并编译 `.mlpackage`）
    pub fn load_model(package_root: &Path) -> Result<Self, AlgoError> {
        let _pool = AutoreleasePool::new();

        // 优先在 model/ 目录下查找 .mlpackage 或 .mlmodel
        let candidate_models = [
            package_root.join("model").join("yolo26n.mlpackage"),
            package_root.join("yolo26n.mlpackage"),
        ];

        let mut resolved_path = None;
        for c in &candidate_models {
            if c.exists() {
                resolved_path = Some(c.clone());
                break;
            }
        }

        // 如果没有预设文件，扫描 model 目录下任意 .mlpackage
        let model_path = match resolved_path {
            Some(p) => p,
            None => {
                let model_dir = package_root.join("model");
                let mut found = None;
                if let Ok(entries) = std::fs::read_dir(&model_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if let Some(ext) = p.extension() {
                            if ext == "mlpackage" || ext == "mlmodel" || ext == "mlmodelc" {
                                found = Some(p);
                                break;
                            }
                        }
                    }
                }
                found.ok_or_else(|| AlgoError::ModelLoad {
                    reason: format!("在 {:?} 目录下未找到 CoreML 模型文件", model_dir),
                })?
            }
        };

        let path_str = model_path.to_str().ok_or_else(|| AlgoError::ModelLoad {
            reason: "模型路径不是合法 UTF-8 编码".to_string(),
        })?;

        let ns_path = ns_string(path_str)?;
        let nsurl_cls = get_class("NSURL")?;
        let file_url_sel = register_sel("fileURLWithPath:")?;

        // SAFETY: ns_path 为有效 NSString
        let model_url = unsafe { msg_send!(nsurl_cls, file_url_sel, ns_path) };
        if model_url.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: "创建模型 NSURL 失败".to_string(),
            });
        }

        let mlmodel_cls = get_class("MLModel")?;

        // 若为源码格式 (.mlpackage 或 .mlmodel)，调用 compileModelAtURL:error:
        let mut compiled_url = model_url;
        if path_str.ends_with(".mlpackage") || path_str.ends_with(".mlmodel") {
            let compile_sel = register_sel("compileModelAtURL:error:")?;
            let mut err: *mut c_void = null_mut();
            // SAFETY: mlmodel_cls, compile_sel, model_url 均有效
            let url = unsafe { msg_send!(mlmodel_cls, compile_sel, model_url, &mut err) };
            if !err.is_null() || url.is_null() {
                return Err(AlgoError::ModelLoad {
                    reason: format!("编译 CoreML 模型失败: {}", ns_error_desc(err)),
                });
            }
            compiled_url = url;
        }

        // 创建 MLModelConfiguration，并配置 computeUnits = 2 (MLComputeUnitsAll: ANE + GPU + CPU)
        // 注意: 0 = MLComputeUnitsCPUOnly, 1 = MLComputeUnitsCPUAndGPU, 2 = MLComputeUnitsAll
        let config_cls = get_class("MLModelConfiguration")?;
        let alloc_sel = register_sel("alloc")?;
        let init_sel = register_sel("init")?;
        // SAFETY: config_cls 构造配置对象
        let config = unsafe { msg_send!(msg_send!(config_cls, alloc_sel), init_sel) };
        let set_units_sel = register_sel("setComputeUnits:")?;
        // SAFETY: 2 = MLComputeUnitsAll
        unsafe { msg_send_void!(config, set_units_sel, 2isize) };

        // 加载模型: +[MLModel modelWithContentsOfURL:configuration:error:]
        let load_sel = register_sel("modelWithContentsOfURL:configuration:error:")?;
        let mut err: *mut c_void = null_mut();
        // SAFETY: 调用模型加载
        let model = unsafe { msg_send!(mlmodel_cls, load_sel, compiled_url, config, &mut err) };
        if !err.is_null() || model.is_null() {
            return Err(AlgoError::ModelLoad {
                reason: format!("加载 CoreML 模型失败: {}", ns_error_desc(err)),
            });
        }

        // SAFETY: 增加对象引用计数，防止自动释放池销毁
        let retained_model = unsafe { objc_retain(model) };

        // 动态自省模型输入与输出特征名称
        let model_desc_sel = register_sel("modelDescription")?;
        let input_desc_sel = register_sel("inputDescriptionsByName")?;
        let output_desc_sel = register_sel("outputDescriptionsByName")?;
        let all_keys_sel = register_sel("allKeys")?;
        let obj_for_key_sel = register_sel("objectForKey:")?;
        let type_sel = register_sel("type")?;
        let count_sel = register_sel("count")?;
        let obj_at_idx_sel = register_sel("objectAtIndex:")?;

        // 默认特征名称 (当反射未匹配时保底)
        let mut input_name_ns = ns_string("image")?;
        let mut output_name_ns = ns_string("var_911")?;

        // SAFETY: 提取模型输入特征
        unsafe {
            let desc = msg_send!(retained_model, model_desc_sel);
            if !desc.is_null() {
                let inputs = msg_send!(desc, input_desc_sel);
                if !inputs.is_null() {
                    let keys = msg_send!(inputs, all_keys_sel);
                    let count = msg_send_isize!(keys, count_sel);
                    for i in 0..count {
                        let key = msg_send!(keys, obj_at_idx_sel, i);
                        let feat = msg_send!(inputs, obj_for_key_sel, key);
                        let t = msg_send_isize!(feat, type_sel);
                        if t == 4 {
                            // 4 = MLFeatureTypeImage
                            input_name_ns = key;
                            break;
                        }
                    }
                }

                let outputs = msg_send!(desc, output_desc_sel);
                if !outputs.is_null() {
                    let keys = msg_send!(outputs, all_keys_sel);
                    let count = msg_send_isize!(keys, count_sel);
                    for i in 0..count {
                        let key = msg_send!(keys, obj_at_idx_sel, i);
                        let feat = msg_send!(outputs, obj_for_key_sel, key);
                        let t = msg_send_isize!(feat, type_sel);
                        if t == 5 {
                            // 5 = MLFeatureTypeMultiArray
                            output_name_ns = key;
                            break;
                        }
                    }
                }
            }
        }

        // SAFETY: 增加对象引用计数，防止自动释放池销毁
        let input_name_ns = unsafe { objc_retain(input_name_ns) };
        // SAFETY: 增加对象引用计数，防止自动释放池销毁
        let output_name_ns = unsafe { objc_retain(output_name_ns) };

        let selectors = CachedSelectors {
            feat_val_cls: get_class("MLFeatureValue")?,
            feat_pixel_sel: register_sel("featureValueWithPixelBuffer:")?,
            dict_cls: get_class("NSDictionary")?,
            dict_sel: register_sel("dictionaryWithObject:forKey:")?,
            provider_cls: get_class("MLDictionaryFeatureProvider")?,
            alloc_sel: register_sel("alloc")?,
            init_dict_sel: register_sel("initWithDictionary:error:")?,
            predict_sel: register_sel("predictionFromFeatures:error:")?,
            feat_for_name_sel: register_sel("featureValueForName:")?,
            multiarray_sel: register_sel("multiArrayValue")?,
            count_sel: register_sel("count")?,
            data_ptr_sel: register_sel("dataPointer")?,
            strides_sel: register_sel("strides")?,
            obj_at_idx_sel: register_sel("objectAtIndex:")?,
            int_val_sel: register_sel("integerValue")?,
            datatype_sel: register_sel("dataType")?,
        };

        Ok(Self {
            model: retained_model,
            input_name_ns,
            output_name_ns,
            selectors,
        })
    }

    /// 执行 CVPixelBuffer 显存零拷贝前向推理
    ///
    /// 输出格式为 `var_911` 张量扁平数据（300 × 6 = 1800 个浮点数）
    ///
    /// # Safety
    /// `cv_pixelbuffer` 必须指向有效且未被释放的 `CVPixelBufferRef` 硬件句柄。
    pub unsafe fn predict_pixelbuffer(
        &self,
        cv_pixelbuffer: *mut c_void,
    ) -> Result<Vec<f32>, AlgoError> {
        if cv_pixelbuffer.is_null() {
            return Err(AlgoError::Internal {
                reason: "输入 CVPixelBuffer 为空".to_string(),
            });
        }

        let _pool = AutoreleasePool::new();
        let s = self.selectors;

        // SAFETY: cv_pixelbuffer 为有效 CVPixelBufferRef
        let feat_val = unsafe { msg_send!(s.feat_val_cls, s.feat_pixel_sel, cv_pixelbuffer) };
        if feat_val.is_null() {
            return Err(AlgoError::Internal {
                reason: "包装 MLFeatureValue(CVPixelBuffer) 失败".to_string(),
            });
        }

        // SAFETY: 构造以 input_name_ns 为键的 NSDictionary
        let dict = unsafe { msg_send!(s.dict_cls, s.dict_sel, feat_val, self.input_name_ns) };
        if dict.is_null() {
            return Err(AlgoError::Internal {
                reason: "构造输入 NSDictionary 失败".to_string(),
            });
        }

        let mut err: *mut c_void = null_mut();
        // SAFETY: 构造 MLDictionaryFeatureProvider
        let provider = unsafe {
            msg_send!(
                msg_send!(s.provider_cls, s.alloc_sel),
                s.init_dict_sel,
                dict,
                &mut err
            )
        };
        if !err.is_null() || provider.is_null() {
            return Err(AlgoError::Internal {
                reason: format!("构造特征 Provider 失败: {}", ns_error_desc(err)),
            });
        }

        // 调用模型推理: -[MLModel predictionFromFeatures:error:]
        let mut err: *mut c_void = null_mut();
        // SAFETY: provider 属于有效 MLFeatureProvider
        let out_provider = unsafe { msg_send!(self.model, s.predict_sel, provider, &mut err) };
        if !err.is_null() || out_provider.is_null() {
            return Err(AlgoError::Internal {
                reason: format!("CoreML 模型推理失败: {}", ns_error_desc(err)),
            });
        }

        // 提取输出特征 -[out_provider featureValueForName:]
        // SAFETY: 提取目标输出
        let out_feat = unsafe { msg_send!(out_provider, s.feat_for_name_sel, self.output_name_ns) };
        if out_feat.is_null() {
            return Err(AlgoError::Internal {
                reason: "未在推理输出中找到目标输出特征".to_string(),
            });
        }

        // 提取 MultiArray -[out_feat multiArrayValue]
        // SAFETY: out_feat 为有效 MLFeatureValue
        let multiarray = unsafe { msg_send!(out_feat, s.multiarray_sel) };
        if multiarray.is_null() {
            return Err(AlgoError::Internal {
                reason: "目标输出特征不是 MLMultiArray".to_string(),
            });
        }

        // SAFETY: multiarray.count
        let count = unsafe { msg_send_isize!(multiarray, s.count_sel) };
        if count != 300 * 6 {
            return Err(AlgoError::Internal {
                reason: format!("CoreML 输出张量元素数量不匹配: 期望 1800, 实际 {count}"),
            });
        }

        // SAFETY: multiarray.dataPointer
        let data_ptr = unsafe { msg_send!(multiarray, s.data_ptr_sel) };
        if data_ptr.is_null() {
            return Err(AlgoError::Internal {
                reason: "CoreML 输出张量 dataPointer 为空".to_string(),
            });
        }

        // SAFETY: 提取 strides[1] 与 strides[2]
        let (stride1, stride2) = unsafe {
            let strides = msg_send!(multiarray, s.strides_sel);
            if strides.is_null() {
                (6isize, 1isize)
            } else {
                let s1_obj = msg_send!(strides, s.obj_at_idx_sel, 1usize);
                let s2_obj = msg_send!(strides, s.obj_at_idx_sel, 2usize);
                let s1 = msg_send_isize!(s1_obj, s.int_val_sel);
                let s2 = msg_send_isize!(s2_obj, s.int_val_sel);
                (s1, s2)
            }
        };

        // SAFETY: multiarray.dataType (65552 = Float16, 65568 = Float32, 65600 = Double)
        let data_type = unsafe { msg_send_isize!(multiarray, s.datatype_sel) };

        let mut output = vec![0.0f32; 1800];

        if data_type == 65552 {
            // Float16 (f16 / IEEE 754 half precision) -> 通过 Accelerate 框架 SIMD/NEON 硬件转换
            if stride1 == 6 && stride2 == 1 {
                let src_buf = VImageBuffer {
                    data: data_ptr,
                    height: 1,
                    width: 1800,
                    row_bytes: 1800 * 2,
                };
                let dest_buf = VImageBuffer {
                    data: output.as_mut_ptr() as *mut c_void,
                    height: 1,
                    width: 1800,
                    row_bytes: 1800 * 4,
                };
                // SAFETY: src_buf 与 dest_buf 分别为合法的 1800 元素 f16 与 f32 缓冲区
                let ret = unsafe { vImageConvert_Planar16FtoPlanarF(&src_buf, &dest_buf, 0) };
                if ret != 0 {
                    return Err(AlgoError::Internal {
                        reason: format!("vImageConvert_Planar16FtoPlanarF 失败: {ret}"),
                    });
                }
            } else {
                let mut contiguous = [0u16; 1800];
                let u16_ptr = data_ptr as *const u16;
                for i in 0..300 {
                    for j in 0..6 {
                        let offset = (i * stride1 + j * stride2) as usize;
                        // SAFETY: offset 在合法 1800 元素数据范围内且 data_ptr 有效
                        contiguous[i as usize * 6 + j as usize] = unsafe { *u16_ptr.add(offset) };
                    }
                }
                let src_buf = VImageBuffer {
                    data: contiguous.as_mut_ptr() as *mut c_void,
                    height: 1,
                    width: 1800,
                    row_bytes: 1800 * 2,
                };
                let dest_buf = VImageBuffer {
                    data: output.as_mut_ptr() as *mut c_void,
                    height: 1,
                    width: 1800,
                    row_bytes: 1800 * 4,
                };
                // SAFETY: contiguous 与 output 均为合法的 1800 元素缓冲区
                let ret = unsafe { vImageConvert_Planar16FtoPlanarF(&src_buf, &dest_buf, 0) };
                if ret != 0 {
                    return Err(AlgoError::Internal {
                        reason: format!("vImageConvert_Planar16FtoPlanarF 失败: {ret}"),
                    });
                }
            }
        } else if data_type == 65568 {
            // Float32
            let float_ptr = data_ptr as *const f32;
            for i in 0..300 {
                for j in 0..6 {
                    let offset = (i * stride1 + j * stride2) as usize;
                    // SAFETY: offset 在 1800 数据范围内且 data_ptr 有效
                    let val = unsafe { *float_ptr.add(offset) };
                    output[i as usize * 6 + j as usize] = val;
                }
            }
        } else if data_type == 65600 {
            // Double
            let double_ptr = data_ptr as *const f64;
            for i in 0..300 {
                for j in 0..6 {
                    let offset = (i * stride1 + j * stride2) as usize;
                    // SAFETY: offset 在 1800 数据范围内且 data_ptr 有效
                    let val = unsafe { *double_ptr.add(offset) as f32 };
                    output[i as usize * 6 + j as usize] = val;
                }
            }
        } else {
            return Err(AlgoError::Internal {
                reason: format!("不支持的 CoreML 张量数据类型: {data_type}"),
            });
        }

        Ok(output)
    }
}

impl Drop for CoreMlRunner {
    fn drop(&mut self) {
        // SAFETY: 释放所持有的 Objective-C 对象
        unsafe {
            if !self.output_name_ns.is_null() {
                objc_release(self.output_name_ns);
            }
            if !self.input_name_ns.is_null() {
                objc_release(self.input_name_ns);
            }
            if !self.model.is_null() {
                objc_release(self.model);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::platforms::apple::AppleCvEngine;
    use algo_sdk::cv::CvEngine;
    use algo_sdk::testing::MockFrameBuilder;

    #[test]
    fn test_coreml_model_load_and_prediction() {
        let pkg_path = Path::new(env!("CARGO_MANIFEST_DIR"));
        let runner = CoreMlRunner::load_model(pkg_path).expect("CoreML 模型加载应成功");

        let frame = MockFrameBuilder::new()
            .dimensions(640, 384)
            .to_nv12(640)
            .opaque_kind(algo_sdk::c_abi::AV_OPAQUE_CVPIXELBUFFER)
            .build();
        let safe = frame.as_safe_frame();

        let (buf, _) = AppleCvEngine
            .letterbox(&safe, 640, 384, [114, 114, 114])
            .expect("硬件预处理应成功");
        let ptr = buf.as_raw_ptr().expect("必须生成 CVPixelBuffer");

        // SAFETY: ptr 来自有效的 CvBuffer
        let out = unsafe {
            runner
                .predict_pixelbuffer(ptr)
                .expect("CoreML 前向推理应成功")
        };
        assert_eq!(out.len(), 1800);
        for val in &out {
            assert!(val.is_finite());
        }
    }
}
