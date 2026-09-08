//! 算法检测结果发射器 (ResultEmitter)
//!
//! 负责将算法计算出的目标框序列化为 Heimdall 契约 JSON，
//! 挂载全景大图/特写抓拍请求，并安全回调宿主 `on_result`。

use std::ffi::{c_void, CString};
use std::marker::PhantomData;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::c_abi::*;
use crate::error::AlgoError;
use crate::math::NormBox;

/// 内部目标对象 JSON 序列化结构
#[derive(Debug, Serialize)]
struct JsonAlarmObject<'a> {
    class_id: u32,
    label: &'a str,
    confidence: f32,
    bbox: [f32; 4],
}

/// 零分配检测框切片序列化代理
#[derive(Debug)]
struct BoxesSerializer<'a>(&'a [NormBox]);

impl<'a> Serialize for BoxesSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for b in self.0 {
            seq.serialize_element(&JsonAlarmObject {
                class_id: b.class_id,
                label: b.label.unwrap_or(""),
                confidence: b.confidence,
                bbox: [b.x, b.y, b.w, b.h],
            })?;
        }
        seq.end()
    }
}

/// 内部告警事件根信封
#[derive(Debug, Serialize)]
struct JsonAlarmEnvelope<'a> {
    event_id: Uuid,
    objects: BoxesSerializer<'a>,
}

/// 算法结果发射器
#[derive(Debug)]
pub struct ResultEmitter<'a> {
    frame_id: u64,
    on_result: Option<AvAlgoResultCb>,
    user_data: *mut c_void,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> ResultEmitter<'a> {
    /// 从 C ABI 回调指针构造
    ///
    /// # Safety
    /// 若 `on_result` 不为空，在生命周期 `'a` 内必须是指向合法宿主回调的函数指针。
    pub unsafe fn from_raw(
        frame_id: u64,
        on_result: Option<AvAlgoResultCb>,
        user_data: *mut c_void,
    ) -> Self {
        Self {
            frame_id,
            on_result,
            user_data,
            _lifetime: PhantomData,
        }
    }

    /// 获取当前正在处理的帧序号
    #[inline]
    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }

    /// 发射指定类型的 JSON 结果。
    ///
    /// `json` 可以不带结尾 NUL；方法会在回调前构造临时 C 字符串，保证
    /// `AvAlgoResult::json` 在宿主同步回调期间有效。
    pub fn emit_json_result(
        &mut self,
        kind: u32,
        json: &[u8],
        images: &[AvAlgoImageReq],
    ) -> Result<(), AlgoError> {
        let json_len = u32::try_from(json.len()).map_err(|_| AlgoError::OutOfMemory)?;
        let image_count = u32::try_from(images.len()).map_err(|_| AlgoError::OutOfMemory)?;
        let c_json = CString::new(json).map_err(|e| AlgoError::Internal {
            reason: format!("结果 JSON 包含非法空字符: {e}"),
        })?;
        let result = AvAlgoResult {
            size: std::mem::size_of::<AvAlgoResult>() as u32,
            api_version: AV_ALGO_API_VERSION,
            kind,
            reserved0: 0,
            frame_id: self.frame_id,
            json: c_json.as_ptr(),
            json_len,
            image_count,
            images: if images.is_empty() {
                std::ptr::null()
            } else {
                images.as_ptr()
            },
        };

        if let Some(cb) = self.on_result {
            // SAFETY: result、c_json 和 images 在同步回调期间保持有效；宿主不得保存裸指针。
            unsafe {
                cb(&result, self.user_data);
            }
        }
        Ok(())
    }

    /// 发射人脸识别结果 JSON，不自动附加抓拍请求。
    pub fn emit_recognition_json(&mut self, json: &[u8]) -> Result<(), AlgoError> {
        self.emit_json_result(AV_RESULT_RECOGNITION, json, &[])
    }

    /// 发射告警检测结果，并自动请求全景大图抓拍
    pub fn emit_detections(&mut self, boxes: &[NormBox]) -> Result<(), AlgoError> {
        let envelope = JsonAlarmEnvelope {
            event_id: Uuid::new_v4(),
            objects: BoxesSerializer(boxes),
        };

        let json_bytes = serde_json::to_vec(&envelope).map_err(|e| AlgoError::Internal {
            reason: format!("序列化告警 JSON 失败: {e}"),
        })?;

        // 默认挂载全景大图抓拍请求 (x=0, y=0, w=1, h=1, purpose=1)
        let full_req = AvAlgoImageReq {
            size: std::mem::size_of::<AvAlgoImageReq>() as u32,
            api_version: AV_ALGO_API_VERSION,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            purpose: 1,
            reserved0: 0,
        };

        self.emit_json_result(AV_RESULT_ALARM, &json_bytes, &[full_req])
    }

    /// 自检模式发射合格信号
    pub fn emit_self_test(&mut self, detection_count: usize) -> Result<(), AlgoError> {
        let json_bytes = serde_json::to_vec(&serde_json::json!({
            "self_test": true,
            "detections_count": detection_count,
            "status": "passed"
        }))
        .map_err(|e| AlgoError::Internal {
            reason: e.to_string(),
        })?;

        self.emit_json_result(AV_RESULT_SELF_TEST, &json_bytes, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_emitter_json_compatibility() {
        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn mock_cb(result: *const AvAlgoResult, _user: *mut c_void) {
            // SAFETY: result 非空
            let res = unsafe { &*result };
            assert_eq!(res.kind, AV_RESULT_ALARM);
            assert_eq!(res.image_count, 1);
            // SAFETY: json 指针合法
            let json_cstr = unsafe { CStr::from_ptr(res.json) };
            let json_str = json_cstr.to_str().expect("合法 UTF-8");

            let parsed: serde_json::Value = serde_json::from_str(json_str).expect("合法 JSON");
            assert!(parsed.get("event_id").is_some());
            let objs = parsed
                .get("objects")
                .and_then(|v| v.as_array())
                .expect("objects 数组");
            assert_eq!(objs.len(), 1);
            assert_eq!(objs[0]["label"], "person");
            assert_eq!(objs[0]["confidence"], 0.95);

            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        // SAFETY: 测试中传入合法的静态 mock_cb 回调函数
        let mut emitter =
            unsafe { ResultEmitter::from_raw(1001, Some(mock_cb), std::ptr::null_mut()) };

        let boxes = [NormBox::new(0.1, 0.2, 0.3, 0.4, 0.95, 0).with_label("person")];
        emitter.emit_detections(&boxes).expect("发射成功");

        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
    }
}
