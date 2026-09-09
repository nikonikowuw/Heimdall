# RK3576 Model Artifacts

These `.rknn` files are deployment artifacts, not source models. Their SHA-256 values are recorded in the package root `manifest.json` and verified before a worker is started.

The conversion scripts, ONNX sources, calibration dataset and verbose RKNN Toolkit2 build reports are not included in this repository. Therefore model conversion is not reproducible from this directory alone. Before changing either artifact, regenerate the manifest hash and verify the input/output contract with board-side `rknn_query`.

Expected board checks:

```bash
sha256sum model/*.rknn
# run the package hardware test after installing the matching librknnrt.so
cargo test -p face-recognition-rknn -- --ignored --nocapture
```

Detector output order is four NCHW branches per scale, in the order `box(64)`, `score(1)`, `class(1)`, `landmark(15)` for `48x80`, `24x40`, and `12x20`. The embedder output is a single 512-element tensor; runtimes may report it as rank 2 or padded rank 4, both are validated against the manifest contract.
