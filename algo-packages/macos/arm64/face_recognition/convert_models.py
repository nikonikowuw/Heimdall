#!/usr/bin/env python3
"""
Convert YOLOv8-face / YOLOv5n-face and EdgeFace weights to Apple CoreML .mlpackage format.

Inputs (in weights/):
  - yolov8-lite-s.pt (or yolov5n-face.pt)
  - edgeface_xs_gamma_06.onnx

Outputs (in model/):
  - yolov8_face.mlpackage (input: 1x3x384x640 RGB [0, 1], output: 1x5040x20 FLOAT16)
  - edgeface_s.mlpackage (input: 1x3x112x112 RGB [-1, 1], output: 1x512 FLOAT16)
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


def ensure_edgeface_weights():
    onnx_path = WEIGHTS_DIR / "edgeface_xs_gamma_06.onnx"
    if not onnx_path.exists():
        WEIGHTS_DIR.mkdir(parents=True, exist_ok=True)
        url = "https://github.com/yakhyo/edgeface-onnx/releases/download/weights/edgeface_xs_gamma_06.onnx"
        print(f"Downloading EdgeFace weights from {url}...")
        subprocess.run(["curl", "-L", url, "-o", str(onnx_path)], check=True)
    return onnx_path


def convert_edgeface():
    print("\n=== [1/2] Converting EdgeFace-s ===")
    onnx_path = ensure_edgeface_weights()
    output_path = MODEL_DIR / "edgeface_s.mlpackage"

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


def convert_yolov8_face():
    print("\n=== [2/2] Converting YOLOv8-face (yolov8-lite-s) ===")
    pt_path = WEIGHTS_DIR / "yolov8-lite-s.pt"
    assert pt_path.exists(), f"Missing PyTorch weights: {pt_path}"
    output_path = MODEL_DIR / "yolov8_face.mlpackage"

    yolov8_repo = Path("/tmp/yolov8-face")
    if not (yolov8_repo / "ultralytics").exists():
        print("Cloning derronqi/yolov8-face repository...")
        subprocess.run(
            ["git", "clone", "--depth", "1", "https://github.com/derronqi/yolov8-face.git", str(yolov8_repo)],
            check=True,
        )

    sys.modules["seaborn"] = MagicMock()
    sys.modules["matplotlib"] = MagicMock()
    sys.modules["matplotlib.pyplot"] = MagicMock()

    if str(yolov8_repo) not in sys.path:
        sys.path.insert(0, str(yolov8_repo))

    import ultralytics.nn.modules.block as block_mod
    # CoreML-friendly channel shuffle without dynamic tensor unpacking
    block_mod.channel_shuffle = lambda x, groups=2: x.unflatten(1, (groups, -1)).transpose(1, 2).flatten(1, 2)

    import coremltools as ct

    print("Step 1: Loading PyTorch yolov8-lite-s checkpoint...")
    ckpt = torch.load(str(pt_path), map_location="cpu", weights_only=False)
    model = ckpt["model"].float()
    for mod in model.modules():
        if isinstance(mod, torch.nn.Conv2d):
            mod.dilation = tuple(int(x) for x in mod.dilation)

    # Static upsample sizes for 640x384 input
    model.model[9] = nn.Upsample(size=(24, 40), mode="nearest")
    model.model[9].f = -1
    model.model[9].i = 9
    model.model[13] = nn.Upsample(size=(48, 80), mode="nearest")
    model.model[13].f = -1
    model.model[13].i = 13

    class StaticPoseDecoder(nn.Module):
        """
        Static ANE-optimized decoder for YOLOv8-face.
        Produces [1, 5040, 20] tensor:
          cx, cy, w, h, cls_conf, 5 * (x, y, landmark_conf)
        """
        def __init__(self, pose_module, img_h=384, img_w=640):
            super().__init__()
            self.cv2 = pose_module.cv2
            self.cv3 = pose_module.cv3
            self.cv4 = pose_module.cv4
            self.dfl_conv = pose_module.dfl.conv
            self.reg_max = pose_module.reg_max

            strides = [8, 16, 32]
            dims = [(48, 80), (24, 40), (12, 20)]
            for i, (stride, (ny, nx)) in enumerate(zip(strides, dims)):
                yv, xv = torch.meshgrid([torch.arange(ny), torch.arange(nx)], indexing="ij")
                ax = (xv.float() + 0.5).view(1, ny * nx)
                ay = (yv.float() + 0.5).view(1, ny * nx)
                self.register_buffer(f"ax_{i}", ax)
                self.register_buffer(f"ay_{i}", ay)
                self.register_buffer(f"stride_{i}", torch.tensor(float(stride)))

        def forward(self, feats):
            outputs = []
            anchor_counts = [3840, 960, 240]
            for i in range(3):
                x = feats[i]
                box_raw = self.cv2[i](x)
                cls_raw = self.cv3[i](x)
                kpt_raw = self.cv4[i](x)
                num_anchors = anchor_counts[i]

                box_reshaped = box_raw.view(1, 4, 16, num_anchors).transpose(2, 1).softmax(1)
                dist = self.dfl_conv(box_reshaped).view(1, 4, num_anchors)
                lt = dist[:, :2]
                rb = dist[:, 2:]

                ax = getattr(self, f"ax_{i}")
                ay = getattr(self, f"ay_{i}")
                stride = getattr(self, f"stride_{i}")

                x1 = ax - lt[:, 0]
                y1 = ay - lt[:, 1]
                x2 = ax + rb[:, 0]
                y2 = ay + rb[:, 1]

                cx = (x1 + x2) * 0.5 * stride
                cy = (y1 + y2) * 0.5 * stride
                w = (x2 - x1) * stride
                h = (y2 - y1) * stride

                score = cls_raw.view(1, 1, num_anchors).sigmoid()

                kpt_flat = kpt_raw.view(1, 15, num_anchors)
                kpts_out = []
                for k in range(5):
                    kx = (kpt_flat[:, k * 3] * 2.0 + ax - 0.5) * stride
                    ky = (kpt_flat[:, k * 3 + 1] * 2.0 + ay - 0.5) * stride
                    kc = kpt_flat[:, k * 3 + 2].sigmoid()
                    kpts_out.extend([kx.unsqueeze(1), ky.unsqueeze(1), kc.unsqueeze(1)])

                kpts_tensor = torch.cat(kpts_out, dim=1)
                scale_out = torch.cat(
                    [cx.unsqueeze(1), cy.unsqueeze(1), w.unsqueeze(1), h.unsqueeze(1), score, kpts_tensor],
                    dim=1,
                )
                outputs.append(scale_out.permute(0, 2, 1))

            return torch.cat(outputs, dim=1)

    class FullModel(nn.Module):
        def __init__(self, m):
            super().__init__()
            self.backbone = m.model[:-1]
            self.save = m.save
            self.pose = StaticPoseDecoder(m.model[22])

        def forward(self, x):
            y = []
            for mod in self.backbone:
                if mod.f != -1:
                    x = y[mod.f] if isinstance(mod.f, int) else [x if j == -1 else y[j] for j in mod.f]
                x = mod(x)
                y.append(x if mod.i in self.save else None)
            return self.pose([y[15], y[18], y[21]])

    print("Step 2: Tracing static YOLOv8-face model (640x384)...")
    full_model = FullModel(model)
    full_model.eval()
    dummy_input = torch.randn(1, 3, 384, 640)
    traced = torch.jit.trace(full_model, dummy_input)

    print("Step 3: Converting to CoreML .mlpackage (ANE/GPU FLOAT16)...")
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
    print(f"Successfully exported YOLOv8-face CoreML model: {output_path}")

    # Maintain yolov5n_face.mlpackage link for seamless backward compatibility
    legacy_path = MODEL_DIR / "yolov5n_face.mlpackage"
    if legacy_path.is_symlink() or legacy_path.exists():
        if legacy_path.is_symlink():
            legacy_path.unlink()
    if not legacy_path.exists():
        legacy_path.symlink_to("yolov8_face.mlpackage")

    # Verification
    test_img = Image.new("RGB", (640, 384), color=(114, 114, 114))
    preds = mlmodel.predict({"image": test_img})
    out = preds["var_911"]
    assert out.shape == (1, 5040, 20), f"Unexpected output shape: {out.shape}"
    print(f"Verification passed: output tensor shape = {out.shape}")


def convert_person_detect():
    print("\n=== [3/3] Converting Person Detector (yolo26n) ===")
    pt_path = WEIGHTS_DIR / "yolo26n.pt"
    if not pt_path.exists():
        fallback_path = Path("/Users/zhang/dev/go/argus/algo-packages/macos/arm64/face_recognition/weights/yolo26n.pt")
        if fallback_path.exists():
            import shutil
            WEIGHTS_DIR.mkdir(parents=True, exist_ok=True)
            shutil.copy(fallback_path, pt_path)
    assert pt_path.exists(), f"Missing PyTorch weights: {pt_path}"

    from ultralytics.nn.modules.block import Attention

    def static_attention_forward(self, x: torch.Tensor) -> torch.Tensor:
        B = 1
        C = int(x.shape[1])
        H = int(x.shape[2])
        W = int(x.shape[3])
        N = H * W
        qkv = self.qkv(x)
        qkv_flat = qkv.view(1, self.num_heads, self.key_dim * 2 + self.head_dim, N)
        q = qkv_flat[:, :, :self.key_dim, :]
        k = qkv_flat[:, :, self.key_dim:self.key_dim * 2, :]
        v = qkv_flat[:, :, self.key_dim * 2:, :]

        attn = ((q * self.scale).transpose(-2, -1) @ k).softmax(dim=-1)
        out = (v @ attn.transpose(-2, -1)).view(1, C, H, W)
        pe = self.pe(v.reshape(1, C, H, W))
        return self.proj(out + pe)

    Attention.forward = static_attention_forward

    import coremltools as ct

    ckpt = torch.load(str(pt_path), map_location="cpu", weights_only=False)
    model = ckpt["model"].float()
    model.eval()

    detect = model.model[23]
    detect.end2end = False
    detect.export = True

    class PersonDetectModel(torch.nn.Module):
        def __init__(self, m):
            super().__init__()
            self.m = m
        def forward(self, x):
            out = self.m(x)
            person_out = torch.cat([out[:, :4, :], out[:, 4:5, :]], dim=1)
            return person_out.permute(0, 2, 1)

    p_model = PersonDetectModel(model)

    # 1. 640x384
    print("Step 1: Converting person_detect_640x384.mlpackage...")
    detect.shape = None
    dummy_640 = torch.randn(1, 3, 384, 640)
    _ = p_model(dummy_640)
    traced_640 = torch.jit.trace(p_model, dummy_640)
    ml_640 = ct.convert(
        traced_640,
        inputs=[ct.ImageType(name="image", shape=(1, 3, 384, 640), color_layout=ct.colorlayout.RGB, scale=1.0/255.0)],
        outputs=[ct.TensorType(name="var_911")],
        compute_precision=ct.precision.FLOAT16,
        compute_units=ct.ComputeUnit.ALL,
        minimum_deployment_target=ct.target.macOS13,
    )
    out_640 = MODEL_DIR / "person_detect_640x384.mlpackage"
    ml_640.save(str(out_640))
    print(f"Successfully exported {out_640}")

    # 2. 384x224 (padded from 384x216, multiple of stride 32)
    print("Step 2: Converting person_detect_384x216.mlpackage (384x224)...")
    detect.shape = None
    dummy_384 = torch.randn(1, 3, 224, 384)
    _ = p_model(dummy_384)
    traced_384 = torch.jit.trace(p_model, dummy_384)
    ml_384 = ct.convert(
        traced_384,
        inputs=[ct.ImageType(name="image", shape=(1, 3, 224, 384), color_layout=ct.colorlayout.RGB, scale=1.0/255.0)],
        outputs=[ct.TensorType(name="var_911")],
        compute_precision=ct.precision.FLOAT16,
        compute_units=ct.ComputeUnit.ALL,
        minimum_deployment_target=ct.target.macOS13,
    )
    out_384 = MODEL_DIR / "person_detect_384x216.mlpackage"
    ml_384.save(str(out_384))
    print(f"Successfully exported {out_384}")

    # Maintain default person_detect.mlpackage symlink to 640x384
    default_link = MODEL_DIR / "person_detect.mlpackage"
    if default_link.is_symlink() or default_link.exists():
        if default_link.is_symlink():
            default_link.unlink()
    if not default_link.exists():
        default_link.symlink_to("person_detect_640x384.mlpackage")

    # Verification
    test_img_640 = Image.new("RGB", (640, 384), color=(114, 114, 114))
    preds_640 = ml_640.predict({"image": test_img_640})
    assert preds_640["var_911"].shape == (1, 5040, 5)

    test_img_384 = Image.new("RGB", (384, 224), color=(114, 114, 114))
    preds_384 = ml_384.predict({"image": test_img_384})
    assert preds_384["var_911"].shape == (1, 1764, 5)
    print("Verification passed for both person detection models!")


def main():
    convert_edgeface()
    if (WEIGHTS_DIR / "yolov8-lite-s.pt").exists():
        convert_yolov8_face()
    else:
        print(f"yolov8-lite-s.pt not found in {WEIGHTS_DIR}")
    if (WEIGHTS_DIR / "yolo26n.pt").exists():
        convert_person_detect()
    else:
        print(f"yolo26n.pt not found in {WEIGHTS_DIR}")
    print("\n All models converted and verified successfully!")


if __name__ == "__main__":
    main()
