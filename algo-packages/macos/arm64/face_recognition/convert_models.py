#!/usr/bin/env python3
"""
Convert YOLOv5n-face and EdgeFace weights to Apple CoreML .mlpackage format.

Inputs (in weights/):
  - edgeface_xs_gamma_06.onnx
  - yolov5n-face.pt

Outputs (in model/):
  - edgeface_s.mlpackage (input: 1x3x112x112 RGB [-1, 1], output: 1x512 FLOAT16)
  - yolov5n_face.mlpackage (input: 1x3x384x640 RGB [0, 1], output: 1x15120x16 FLOAT16)
"""

import os
import sys
import tempfile
import subprocess
from pathlib import Path
from unittest.mock import MagicMock

import torch
import torch.nn as nn
import numpy as np
from PIL import Image

PACKAGE_DIR = Path(__file__).resolve().parent
WEIGHTS_DIR = PACKAGE_DIR / "weights"
MODEL_DIR = PACKAGE_DIR / "model"


def convert_edgeface():
    print("\n=== [1/2] Converting EdgeFace-s ===")
    onnx_path = WEIGHTS_DIR / "edgeface_xs_gamma_06.onnx"
    output_path = MODEL_DIR / "edgeface_s.mlpackage"
    assert onnx_path.exists(), f"Missing ONNX weights: {onnx_path}"

    import onnx
    import onnxslim
    import onnx2torch
    import coremltools as ct

    print("Step 1: Fixing ONNX Clip empty string inputs...")
    onnx_model = onnx.load(str(onnx_path))
    for node in onnx_model.graph.node:
        if node.op_type == "Clip" and len(node.input) == 3 and node.input[2] == "":
            del node.input[2]

    with tempfile.NamedTemporaryFile(suffix=".onnx", delete=False) as tmp_fixed:
        fixed_path = tmp_fixed.name
    with tempfile.NamedTemporaryFile(suffix=".onnx", delete=False) as tmp_slim:
        slim_path = tmp_slim.name

    try:
        onnx.save(onnx_model, fixed_path)

        print("Step 2: Optimizing ONNX with onnxslim (static input shape [1, 3, 112, 112])...")
        onnxslim.slim(
            fixed_path,
            slim_path,
            input_shapes=["input:1,3,112,112"],
            model_check=True,
        )

        print("Step 3: Loading ONNX into PyTorch GraphModule...")
        torch_model = onnx2torch.convert(slim_path)
        torch_model.eval()

        print("Step 4: Tracing PyTorch model...")
        dummy_input = torch.randn(1, 3, 112, 112)
        traced = torch.jit.trace(torch_model, dummy_input)

        print("Step 5: Converting to CoreML .mlpackage (ANE/GPU FLOAT16)...")
        mlmodel = ct.convert(
            traced,
            inputs=[
                ct.ImageType(
                    name="input",
                    shape=(1, 3, 112, 112),
                    color_layout=ct.colorlayout.RGB,
                    bias=[-1.0, -1.0, -1.0],
                    scale=1.0 / 127.5,
                )
            ],
            outputs=[ct.TensorType(name="embedding")],
            compute_precision=ct.precision.FLOAT16,
            compute_units=ct.ComputeUnit.ALL,
            minimum_deployment_target=ct.target.macOS13,
        )

        MODEL_DIR.mkdir(parents=True, exist_ok=True)
        mlmodel.save(str(output_path))
        print(f"Successfully exported EdgeFace CoreML model: {output_path}")

        # Verification
        test_img = Image.new("RGB", (112, 112), color=(128, 128, 128))
        preds = mlmodel.predict({"input": test_img})
        emb = preds["embedding"]
        assert emb.shape == (1, 512), f"Unexpected embedding shape: {emb.shape}"
        print(f"Verification passed: output embedding shape = {emb.shape}")

    finally:
        if os.path.exists(fixed_path):
            os.remove(fixed_path)
        if os.path.exists(slim_path):
            os.remove(slim_path)


def convert_yolov5_face():
    print("\n=== [2/2] Converting YOLOv5n-face ===")
    pt_path = WEIGHTS_DIR / "yolov5n-face.pt"
    output_path = MODEL_DIR / "yolov5n_face.mlpackage"
    assert pt_path.exists(), f"Missing PyTorch weights: {pt_path}"

    yolov5_repo = Path("/tmp/yolov5-face")
    if not (yolov5_repo / "models" / "yolo.py").exists():
        print("Cloning deepcam-cn/yolov5-face repository...")
        subprocess.run(
            ["git", "clone", "--depth", "1", "https://github.com/deepcam-cn/yolov5-face.git", str(yolov5_repo)],
            check=True,
        )

    sys.modules["seaborn"] = MagicMock()
    sys.modules["matplotlib"] = MagicMock()
    sys.modules["matplotlib.pyplot"] = MagicMock()

    if str(yolov5_repo) not in sys.path:
        sys.path.insert(0, str(yolov5_repo))

    orig_load = torch.load
    torch.load = lambda *args, **kwargs: orig_load(*args, **{**kwargs, "weights_only": False})

    from models.experimental import attempt_load
    import coremltools as ct

    print("Step 1: Loading PyTorch YOLOv5n-face checkpoint...")
    base_model = attempt_load(str(pt_path), map_location="cpu")
    base_model.eval()

    class YOLOv5FaceDecoded(nn.Module):
        """
        Wrapper that decodes YOLOv5-face anchors and produces a single tensor
        of shape [1, N, 16] with layout:
          cx, cy, w, h, obj_conf, cls_conf,
          lm1_x, lm1_y, lm2_x, lm2_y, lm3_x, lm3_y, lm4_x, lm4_y, lm5_x, lm5_y
        Precomputes grid coordinates for 640x384 (Surveillance 16:9 widescreen) to run purely statically on ANE.
        """
        def __init__(self, model):
            super().__init__()
            self.base = model
            detect = model.model[-1]
            self.nl = detect.nl
            self.na = detect.na
            self.no = detect.no
            self.stride = detect.stride

            for i in range(self.nl):
                stride = int(self.stride[i].item())
                ny, nx = 384 // stride, 640 // stride
                yv, xv = torch.meshgrid([torch.arange(ny), torch.arange(nx)], indexing="ij")
                grid = torch.stack((xv, yv), 2).view((1, 1, ny, nx, 2)).expand((1, self.na, ny, nx, 2)).float()
                anchor_grid = (detect.anchors[i].clone() * self.stride[i]).view((1, self.na, 1, 1, 2)).expand((1, self.na, ny, nx, 2)).float()
                self.register_buffer(f"grid_{i}", grid)
                self.register_buffer(f"anchor_grid_{i}", anchor_grid)

        def forward(self, x):
            y = []
            for m in self.base.model:
                if m.f != -1:
                    x = y[m.f] if isinstance(m.f, int) else [x if j == -1 else y[j] for j in m.f]
                if isinstance(m, type(self.base.model[-1])):
                    outputs = []
                    for i in range(self.nl):
                        xi = m.m[i](x[i])
                        bs, _, ny, nx = xi.shape
                        xi = xi.view(bs, self.na, self.no, ny, nx).permute(0, 1, 3, 4, 2).contiguous()

                        grid = getattr(self, f"grid_{i}")
                        anchor_grid = getattr(self, f"anchor_grid_{i}")
                        stride = self.stride[i]

                        box_xy = (xi[..., 0:2].sigmoid() * 2.0 - 0.5 + grid) * stride
                        box_wh = (xi[..., 2:4].sigmoid() * 2.0) ** 2 * anchor_grid
                        obj_conf = xi[..., 4:5].sigmoid()
                        cls_conf = xi[..., 15:16].sigmoid()

                        lm0 = xi[..., 5:7] * anchor_grid + grid * stride
                        lm1 = xi[..., 7:9] * anchor_grid + grid * stride
                        lm2 = xi[..., 9:11] * anchor_grid + grid * stride
                        lm3 = xi[..., 11:13] * anchor_grid + grid * stride
                        lm4 = xi[..., 13:15] * anchor_grid + grid * stride

                        row = torch.cat([box_xy, box_wh, obj_conf, cls_conf, lm0, lm1, lm2, lm3, lm4], dim=-1)
                        outputs.append(row.view(bs, -1, 16))
                    return torch.cat(outputs, dim=1)
                x = m(x)
                y.append(x if m.i in self.base.save else None)

    print("Step 2: Instantiating decoded model with precomputed static grids...")
    decoded_model = YOLOv5FaceDecoded(base_model)
    decoded_model.eval()

    print("Step 3: Tracing PyTorch model (640x384)...")
    dummy_input = torch.randn(1, 3, 384, 640)
    traced = torch.jit.trace(decoded_model, dummy_input)

    print("Step 4: Converting to CoreML .mlpackage (ANE/GPU FLOAT16)...")
    mlmodel = ct.convert(
        traced,
        inputs=[
            ct.ImageType(
                name="image",
                shape=(1, 3, 384, 640),
                color_layout=ct.colorlayout.RGB,
                bias=[0.0, 0.0, 0.0],
                scale=1.0 / 255.0,
            )
        ],
        outputs=[ct.TensorType(name="var_911")],
        compute_precision=ct.precision.FLOAT16,
        compute_units=ct.ComputeUnit.ALL,
        minimum_deployment_target=ct.target.macOS13,
    )

    MODEL_DIR.mkdir(parents=True, exist_ok=True)
    mlmodel.save(str(output_path))
    print(f"Successfully exported YOLOv5n-face CoreML model: {output_path}")

    # Verification
    test_img = Image.new("RGB", (640, 384), color=(114, 114, 114))
    preds = mlmodel.predict({"image": test_img})
    out = preds["var_911"]
    assert out.shape == (1, 15120, 16), f"Unexpected output shape: {out.shape}"
    print(f"Verification passed: output tensor shape = {out.shape}")


def main():
    convert_edgeface()
    convert_yolov5_face()
    print("\n All models converted and verified successfully!")


if __name__ == "__main__":
    main()
