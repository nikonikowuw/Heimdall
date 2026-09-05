# API Specification: 算法包管理与算法实例接口 (API Contract)

所有接口统一基于 `/api/v1`，请求与响应体均采用 `camelCase` 命名的 JSON，统一封装在标准响应信封中：
```json
{
  "code": 0,
  "message": "success",
  "data": ...,
  "timestamp": 1725518400000
}
```

---

## 1. 算法仓库接口 (Algorithm Management)

### 1.1 分页查询算法列表
- **路径**：`GET /api/v1/algorithms`
- **查询参数**：
  - `page`: 可选，数字（默认 1）
  - `pageSize`: 可选，数字（默认 20）
  - `keyword`: 可选，字符串（匹配 `algorithmId` 或 `name`）
  - `algorithmType`: 可选，字符串（如 `object_detection`, `face_recognition`, `license_plate_recognition`）
  - `isBuiltin`: 可选，布尔值（过滤内置或自定义算法）
- **响应数据 `data`**：
```json
{
  "items": [
    {
      "id": 1,
      "algorithmId": "general_detection",
      "name": "General Object Detection",
      "algorithmType": "object_detection",
      "alarmTypeId": "object_detect",
      "activeVersion": "1.0.0",
      "description": "Built-in general object detection model powered by CoreML on Apple Silicon",
      "isBuiltin": true,
      "createdAt": 1725518400000,
      "updatedAt": 1725518400000,
      "versions": [
        {
          "id": 1,
          "algorithmId": "general_detection",
          "version": "1.0.0",
          "platformId": "macos-arm64-coreml",
          "minAdapterVersion": "1.0.0",
          "packageRoot": "algo-packages/macos-arm64/general_detection",
          "fpsTiers": [
            { "fps": 5, "units": 60 },
            { "fps": 15, "units": 150 },
            { "fps": 30, "units": 300 }
          ],
          "configSchema": {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "properties": { "confidence_threshold": { "type": "number", "default": 0.45 } }
          },
          "manifestRaw": {},
          "packageSizeBytes": 45283920,
          "isActive": true,
          "isBuiltin": true,
          "createdAt": 1725518400000,
          "updatedAt": 1725518400000
        }
      ]
    }
  ],
  "total": 1
}
```

### 1.2 查询单个算法详情
- **路径**：`GET /api/v1/algorithms/{id}`
- **响应数据 `data`**：同上述单个 `AlgorithmItem`（包含其版本列表 `versions`）。

### 1.3 查询指定算法的版本列表
- **路径**：`GET /api/v1/algorithms/{id}/versions`
- **响应数据 `data`**：`AlgorithmVersionItem[]`。

### 1.4 上传并安装算法包
- **路径**：`POST /api/v1/algorithms/upload`
- **Content-Type**：`multipart/form-data`
- **字段**：`file`（文件二进制，支持 `.tar.gz`, `.tar`, `.zip`）
- **处理过程**：
  1. 解压至临时目录并防 Zip-Slip 路径校验；
  2. 读取 `manifest.json` 与平台架构匹配；
  3. 执行 7 步安全物理沙箱自测；
  4. 移动至 `var/packages/{algorithm_id}/{version}`；
  5. 写入 `algorithms` 与 `algorithm_versions`，默认设为当前激活版本；
  6. 内存加载至 `AlgoRegistry`。
- **响应数据 `data`**：
```json
{
  "passed": true,
  "stepsTotal": 7,
  "stepsPassed": 7,
  "steps": [
    "1. 路径防穿透与目录结构检查",
    "2. SHA256 完整性与安全指纹校验",
    "3. 解析 Manifest 与平台拓扑匹配",
    "4. Config Schema 参数格式校验",
    "5. 派生隔离子进程与超时守护",
    "6. 算法库 C ABI 导出符号核对",
    "7. 真实前向推理自测与内存复核"
  ],
  "version": {
    "algorithmId": "custom_yolo",
    "version": "1.0.0",
    "platformId": "macos-arm64-coreml",
    "packageRoot": "var/packages/custom_yolo/1.0.0",
    "isActive": true
  }
}
```

### 1.5 激活/回滚指定版本
- **路径**：`PUT /api/v1/algorithms/{id}/versions/{version}/activate`
- **响应数据 `data`**：`null`（成功）。
- **副作用**：触发 `PipelineManager` 优雅热重载所有使用该 `algorithmId` 的在线实例。

### 1.6 卸载指定版本
- **路径**：`DELETE /api/v1/algorithms/{id}/versions/{version}`
- **校验逻辑**：
  - 若 `is_builtin == true`：返回错误码 `403` / `BUILTIN_ALGO_PROTECTED`；
  - 若活跃实例正在使用：返回错误码 `409` / `ALGO_IN_USE`；
- **响应数据 `data`**：`null`（删除成功，物理目录同步清理）。

---

## 2. 算法实例管理接口 (Algorithm Instances)

### 2.1 查询摄像头的算法实例列表
- **路径**：`GET /api/v1/tasks/instances?cameraId={cameraId}`
- **响应数据 `data`**：
```json
[
  {
    "id": 1,
    "instanceId": "550e8400-e29b-41d4-a716-446655440000",
    "cameraId": "cam_gate_01",
    "algorithmId": "general_detection",
    "analysisFps": 15,
    "params": {
      "confidenceThreshold": 0.5,
      "targetClasses": ["person", "car"]
    },
    "rules": [
      {
        "role": "roi",
        "points": [{ "x": 0.1, "y": 0.1 }, { "x": 0.9, "y": 0.9 }]
      }
    ],
    "motionGate": {
      "enabled": true,
      "threshold": 25,
      "contourArea": 100
    },
    "enabled": true,
    "actualStatus": 1,
    "statusMessage": "Running",
    "createdAt": 1725518400000,
    "updatedAt": 1725518400000
  }
]
```

### 2.2 创建算法实例
- **路径**：`POST /api/v1/tasks/instances`
- **请求体 `body`**：
```json
{
  "cameraId": "cam_gate_01",
  "algorithmId": "general_detection",
  "analysisFps": 15,
  "params": { "confidenceThreshold": 0.5 },
  "rules": [],
  "motionGate": { "enabled": true },
  "enabled": true
}
```
- **响应数据 `data`**：创建成功的实例对象（包含生成的 `instanceId`）。

### 2.3 更新算法实例配置
- **路径**：`PUT /api/v1/tasks/instances/{instanceId}`
- **请求体 `body`**：包含 `analysisFps`、`params`、`rules`、`motionGate` 等可修改字段。
- **响应数据 `data`**：更新后的实例对象。

### 2.4 启停算法实例
- **路径**：`PUT /api/v1/tasks/instances/{instanceId}/enabled`
- **请求体 `body`**：`{ "enabled": true }` 或 `{ "enabled": false }`
- **响应数据 `data`**：`null`。

### 2.5 删除算法实例
- **路径**：`DELETE /api/v1/tasks/instances/{instanceId}`
- **响应数据 `data`**：`null`。
- **副作用**：管线销毁对应推理句柄。
