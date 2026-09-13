# Design: Face Recognition Analysis Package

> **Status**: implemented; macOS hardware runtime validated
> **Scope**: macOS arm64 `face_recognition` algorithm package and its existing `infer`/`pipeline` integration
> **Related specifications**: [Media Pipeline](../backend/media-pipeline.md), [Algorithm SDK](../backend/algo-sdk-guidelines.md), [Detection Contract](../backend/detection-alarm-contract.md), [Concurrency](../backend/concurrency-guidelines.md)

## 1. Goals

1. Keep camera stream selection in the host. `main`, `sub`, and `auto` are resolved by the camera pipeline; the algorithm package receives the selected native `FrameRef` and never silently switches streams.
2. Use the CoreML `yolo26n.mlpackage` body detector and the `yolov8_face.mlpackage` face detector on the same 640x384 preprocessing surface.
3. Keep the host `PipelineManager` as the only owner of the cross-layer `trackId`, trajectory, rule state, and capture cooldown. The package may keep a private short-lived tracker only to decide whether a frame is a best-shot candidate; that private ID is never exported.
4. Make the plugin result compatible with the standard detection parser. A per-frame result must not contain plugin-local tracks, `trackId`, pseudo-body flags, or event IDs. An optional Base64 embedding is allowed only on a best-shot object and is consumed by the backend sidecar parser; it is never copied into a frontend DTO.
5. Keep embedding extraction on a low-frequency best-shot path. The package may perform one device-side five-point affine warp and synchronous EdgeFace inference after quality gating, then emit the normalized 512D vector as a little-endian Float32 Base64 string for the host process only. Normal frames omit the field. The host performs 1:N matching in backend memory; embeddings never go to the browser.
6. Preserve the existing evidence and recognition workflow: capture persistence is independent from recognition persistence, and failures do not erase the capture fact.

## 2. Non-goals

- The plugin does not expose its private best-shot tracker. The host `PipelineManager` still owns the public cross-frame track namespace.
- The plugin does not return a 512-dimensional vector on ordinary frames. A best-shot object may carry one backend-only Base64 sidecar.
- The plugin does not choose a camera stream or impose a global model input size on the host.
- The frontend does not receive raw embeddings, landmarks, model tensors, or plugin-local timing breakdowns.

## 3. Runtime Topology

```text
Camera StreamMode(main/sub/auto)
        |
        v
StreamHub -> selected stream decoder -> native FrameRef
        |
        v
face_recognition::instance_process (dedicated inference worker)
        |
        +-- AppleCvEngine letterbox 640x384
        +-- yolo26n: body candidates
        +-- yolov8_face: face candidates + landmarks
        +-- frame-local body/face spatial association
        +-- quality gate
        +-- best-shot gate (low frequency)
        +-- Core Image affine warp on native CVPixelBuffer
        +-- EdgeFace: CVPixelBuffer -> CoreML/ANE (synchronous)
        |
        v
AV_RESULT_RECOGNITION JSON: schema_version + objects[]
        |
        v
infer parser -> Detection -> PipelineManager::SimpleTracker
        |
        +-- host TrackDto / sparse live metadata
        +-- recognition capture rule + bounded evidence task
                  |
                  +-- snapshot_readback_path saves evidence JPEG
                  +-- av_algo_extract_face(JPEG) on a blocking inference worker
                  +-- in-memory gallery 1:N matching
                  +-- Recognition record and sparse business event
```

### Stream policy

The system does not assume that a sub-stream is suitable for face recognition. The camera/task configuration decides the effective analysis stream. `main` is recommended when the sub-stream bitrate destroys facial detail; `sub` remains available for low-cost deployments; `auto` probes and falls back according to the existing pipeline contract. This is a host/media concern, not an algorithm-package concern.

## 4. Ownership and State

| Concern | Owner | Contract |
| --- | --- | --- |
| Camera stream selection | `pipeline` / `media` | `StreamMode` and `StreamHub`; no hidden selection in plugin |
| Frame lifetime and device handle | `types` / `algo-sdk` | borrowed for synchronous `process` call; no raw pointer retained |
| Body/face association | plugin | current-frame spatial matching |
| Cross-frame public `trackId` and trajectory | `pipeline::SimpleTracker` | one host track namespace per camera and algorithm instance |
| Private best-shot gating key | plugin | internal only; never serialized |
| Capture cooldown and rules | `pipeline::RuleEvaluator` | bounded state and source-frame timestamp |
| Feature extraction | plugin best-shot sidecar; `av_algo_extract_face` for enrollment/legacy fallback | low-frequency path; complete 512D normalized vector stays in backend memory |
| 1:N search | `api::gallery_index` | backend-only; no frontend embedding exposure |
| Evidence image and recognition record | `pipeline` / `api` / `db` | bounded async persistence and existing atomic storage rules |

The plugin may use temporary local variables for association and quality calculation. It must not expose its local association state as a public `tracks` array. The host-generated `trackId` is the only ID sent through `PipelineTrackEvent` and UI DTOs.

## 5. Plugin Result Contract

`instance_process()` emits one standard recognition detection envelope. It is intentionally smaller than the previous diagnostic envelope:

```json
{
  "schema_version": 1,
  "objects": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.864,
      "bbox": [0.1200, 0.3630, 0.2800, 0.8500],
      "face": {
        "bbox": [0.1500, 0.3800, 0.2200, 0.5100],
        "confidence": 0.920,
        "quality_score": 0.843,
        "embedding": "dHkevWKSYDxR4/K8..."
      }
    }
  ]
}
```

Contract details:

- `bbox` is normalized `[x1, y1, x2, y2]`, representing the person body bounding box.
- `label: "person"` and `bbox` anchor host tracking (SimpleTracker) with continuous human body IoU continuity, preventing track splitting when faces turn or become occluded.
- `face` is an optional nested object representing the associated face target that passed quality gating.
  - `face.bbox` is normalized `[x1, y1, x2, y2]` of the face.
  - `face.confidence` is the face detection confidence.
  - `face.quality_score` is optional and bounded to `[0.0, 1.0]`. It is a hint for host capture/recognition policy, not an identity result.
  - `face.embedding` is optional and appears only on the first quality-gated best-shot for a new private track. A single-frame local run has no prior track state, so every eligible face is a first best-shot and attempts device-side EdgeFace immediately. It is Base64 of exactly 512 little-endian Float32 values. The host decodes it into an in-memory sidecar, then strips it from all WebSocket/HTTP DTOs.
- Empty `objects` is a valid successful frame with no eligible person target.
- When an identified person has no face detected or fails face quality, `face` is omitted (`None`).
- There is no `tracks`, `trackId`, `eventId`, `isPseudoBody`, `embeddingHead`, or latency object in this payload.
- The host parser remains backward compatible with legacy `faces` and transitional flat `face_bbox` during migration, but canonical package output nests facial metadata inside `face`.

## 6. Low-Frequency Recognition

The EdgeFace model is loaded in the shared CoreML model holder because the same package serves resident detection, best-shot extraction, and the optional enrollment ABI. `instance_process()` does not call the embedder on every frame. This is intentional:

1. Detection/association runs on every sampled frame and must keep predictable latency.
2. The private best-shot state and quality gate reduce repeated embedding work without exporting a duplicate public track ID.
3. On the first best-shot frame of a private track, the package synchronously maps the face landmarks to a source-top-left -> 112x112 target-top-left affine matrix, lets Core Image render directly from the native `CVPixelBuffer` to a 112x112 BGRA surface, and passes that surface directly to EdgeFace. A single-frame local run treats every quality-gated face as a first best-shot. No full-frame D2H readback, CPU RGB repacking, or JPEG round trip occurs.
4. The normalized `[f32; 512]` is serialized once as Base64 across the package/host C ABI JSON callback. The infer parser decodes it into a bounded in-memory sidecar; ordinary detections use `None`.
5. The host `PipelineManager` transfers the sidecar to the capture event. `CaptureDispatchService` uses it directly for gallery search and falls back to `av_algo_extract_face(JPEG)` only for legacy packages that omit the sidecar.
6. The frontend receives only recognition business fields such as subject ID, subject name, similarity, status, and evidence references.

### 6.1 Temporal Spherical Feature Fusion & Anti-Drift Defense

To improve recognition accuracy and signal-to-noise ratio in dynamic edge surveillance, `BestShotManager` introduces a temporal feature fusion strategy:

- **Temporal Gating & Budget**: Feature extraction triggers on the first quality-gated face. Subsequent extractions for the same track occur only when face quality significantly improves ($\Delta Q > 0.08$) or when at least 6 frames have elapsed with a qualified quality score ($Q \ge 0.50$), capped at a maximum of 4 fused frames per track to preserve edge NPU throughput.
- **Hyperspherical Weighted Aggregation**: Extracted 512-dimensional vectors are incrementally weighted by $w_i = Q_i^2$ and reprojected to the unit hypersphere:
  $$\mathbf{v}_{\text{fused}} = \frac{\sum_{i=1}^M w_i \cdot \mathbf{v}_i}{\left\|\sum_{i=1}^M w_i \cdot \mathbf{v}_i\right\|_2}$$
- **Anti-Drift Outlier Defense**: Before fusing a newly extracted embedding into the accumulated centroid, cosine similarity is evaluated: $\cos(\mathbf{v}_{\text{new}}, \mathbf{v}_{\text{fused}}) \ge 0.55$. If the similarity falls below this threshold (indicating tracking drift, severe occlusions, or identity swap), the candidate is rejected, preventing feature pool contamination.

The model call and device-side warp are synchronous inside the dedicated algorithm worker. The system-level persistence and gallery work may remain asynchronous after the detection result is accepted; it must not detach the pixel buffer or enqueue a second host-side pixel pipeline.

## 7. Performance and Backpressure

- The resident path preserves native frame handles through the ABI and uses the package's platform preprocessing. No CPU pixel readback is introduced for ordinary `instance_process` frames.
- A best-shot keeps the source as a native `CVPixelBuffer`: Core Image performs the affine crop/warp into a small device-side BGRA surface, and CoreML consumes that surface synchronously. This is a device-side render into a model input surface, not a claim that crop/resize requires no destination write.
- The selected stream controls the quality/compute trade-off. Face recognition deployments should use a sufficiently high-bitrate main stream when the sub-stream is visibly destructive.
- The host continues to use bounded frame/event queues and drop-oldest behavior. Recognition work is protected by the existing single-flight lock and bounded capture persistence flow.
- Low-frequency evidence readback/JPEG encoding is allowed only after capture selection. It must not be confused with the resident inference path.
- Performance output from `run_local --benchmark` is diagnostic text only and is not part of the C ABI result contract.

## 8. Error and Compatibility Rules

- Missing/corrupt CoreML packages fail model initialization and therefore fail sandbox self-test; the package is not accepted with a partially readable model.
- Invalid model output, invalid coordinates, or a non-finite quality value causes the frame result to fail rather than silently producing a zero box.
- An empty valid detection result is different from a model/ABI failure.
- The optional extraction ABI remains available for enrollment and low-frequency evidence recognition. Its borrowed output is copied by the host before the call returns.
- Existing stream, capture, recognition, and database APIs remain unchanged in this change; only the plugin's per-frame payload is narrowed.

## 9. Acceptance Tests

1. Plugin payload contains `schema_version` and `objects`, uses xyxy coordinates, contains no public `tracks`, `track_id`, or `is_pseudo_body`, and emits `embedding` only on a best-shot object.
2. A face/body candidate is converted from internal xywh to normalized xyxy before emission.
3. Quality-gated face candidates become host `Detection` values with `quality_score`; empty eligible candidates emit a valid empty result.
4. Host `SimpleTracker` is the only source of `trackId` in `PipelineTrackEvent`.
5. `extract_face` still returns a complete normalized 512D vector for enrollment/legacy fallback, while ordinary per-frame results omit the vector and best-shot vectors remain backend-only.
6. Model package integrity checks reject an `.mlpackage` whose `Manifest.json` references missing weights.
7. macOS hardware tests stay `#[ignore]` on non-macOS hosts; pure parsing and geometry tests run in the normal workspace gate.

## 10. Implementation Notes

The implementation deliberately does not expose embeddings to browser JSON. The only per-frame serialization is an optional Base64 sidecar on a best-shot object; the host decodes it into a private in-memory type with a fixed 512-float bound. A future process-separated gallery should use a versioned binary/gRPC contract rather than changing the browser-facing stream payload.
