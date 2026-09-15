#!/usr/bin/env python3
"""
Convert the yolov8n-face ONNX model to RKNN format.

Usage:
    python3 convert_yolov8n_face.py <onnx_model_path> <platform> [dtype] [output_rknn_path] --dataset <calibration.txt>

dtype:
    i8 / u8  INT8 量化
    fp       浮点（不量化）
    mixed    INT8 量化 + auto_hybrid 混合精度

Examples:
    # 视频流模型 640x384
    python3 convert_yolov8n_face.py yolov8n-face-640x384.onnx rk3568 mixed \\
        ../model/yolov8n-face-640x384_rk3568_mixed_face.rknn --dataset calib_merged.txt

    # 注册底库模型 640x640
    python3 convert_yolov8n_face.py yolov8n-face-640x640.onnx rk3568 mixed \\
        ../model/yolov8n-face-640x640_rk3568_mixed_face.rknn --dataset calib_merged.txt

校准集要求（重要）:
    量化校准集必须同时覆盖三个检测尺度（stride 8 / 16 / 32）。
    只用近距离人脸校准集时，stride 8 的分数分支校准范围会被钉在低值区，
    推理阶段真实分数（可达 0.8 以上）会被量化截断到 0.5 附近，
    外部表现就是"人脸置信度恒为 0.5"。推荐合并集：
        <rknn_model_zoo>/datasets/COCO/coco_subset_20.txt  (20 张通用场景)
      + <rknn_model_zoo>/datasets/face_calibration        (14 张人脸场景)
    合并后需按 <rknn_model_zoo> 中的绝对路径或相对各自目录的正确相对路径书写。
"""

import argparse
import hashlib
import os
import sys
from pathlib import Path

from rknn.api import RKNN

DEFAULT_QUANT = True
VALID_DTYPES = ("i8", "u8", "fp", "mixed")


def parse_arg():
    parser = argparse.ArgumentParser(
        usage="%(prog)s onnx_model_path [platform] [dtype] [output_rknn_path] --dataset calibration.txt",
        description="Convert yolov8n-face ONNX to RKNN (RKNN-Toolkit2).",
    )
    parser.add_argument("model_path", help="输入 ONNX 路径")
    parser.add_argument(
        "platform",
        help="目标平台，可选 [" + ", ".join(
            ["rk3562", "rk3566", "rk3568", "rk3576", "rk3588", "rv1126b", "rv1109", "rv1126", "rk1808"]
        ) + "]",
    )
    parser.add_argument("dtype", nargs="?", default="mixed", help="i8 / u8 / fp / mixed（默认 mixed）")
    parser.add_argument("output_path", nargs="?", default=None, help="输出 .rknn 路径")
    parser.add_argument(
        "--dataset",
        default=None,
        help="量化校准清单（每行一个图像路径）；量化时必填，必须覆盖 stride 8/16/32 三个尺度",
    )
    args = parser.parse_args()

    if args.dtype not in VALID_DTYPES:
        parser.error("无效 dtype: {}（可选 {}）".format(args.dtype, "/".join(VALID_DTYPES)))

    do_quant = args.dtype != "fp"
    auto_hybrid = args.dtype == "mixed"

    if do_quant and not args.dataset:
        parser.error(
            "量化转换必须显式提供 --dataset 校准清单；"
            "校准集需覆盖 stride 8/16/32 三个尺度，否则分数分支会被量化截断"
        )

    dataset = None
    if do_quant:
        dataset = Path(args.dataset).expanduser()
        if not dataset.is_file():
            parser.error("校准清单不存在: {}".format(dataset))

    model_path = Path(args.model_path).expanduser()
    if not model_path.is_absolute():
        model_path = Path.cwd() / model_path
    if not model_path.is_file():
        parser.error("ONNX 模型不存在: {}".format(model_path))

    output_path = args.output_path
    if output_path is None:
        output_path = str(model_path.with_suffix(".rknn"))
    elif not Path(output_path).is_absolute():
        output_path = str(Path.cwd() / output_path)

    return model_path, args.platform, do_quant, auto_hybrid, dataset, output_path


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


if __name__ == "__main__":
    model_path, platform, do_quant, auto_hybrid, dataset, output_path = parse_arg()

    print("--> Config model")
    rknn = RKNN(verbose=False)
    # yolov8 使用 RGB 输入并归一化到 [0, 1]（mean=0, std=255）
    ret = rknn.config(
        mean_values=[[0, 0, 0]], std_values=[[255, 255, 255]], target_platform=platform
    )
    if ret != 0:
        print("Config model failed!")
        sys.exit(ret)
    print("done")

    print("--> Loading model")
    ret = rknn.load_onnx(model=str(model_path))
    if ret != 0:
        print("Load model failed!")
        sys.exit(ret)
    print("done")

    print("--> Building model")
    print("   quantization: {}".format("INT8 (auto_hybrid)" if auto_hybrid else ("INT8" if do_quant else "float")))
    if dataset is not None:
        print("   dataset: {}".format(dataset))
    ret = rknn.build(
        do_quantization=do_quant,
        dataset=str(dataset) if dataset is not None else None,
        auto_hybrid=auto_hybrid,
    )
    if ret != 0:
        print("Build model failed!")
        sys.exit(ret)
    print("done")

    print("--> Export rknn model")
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    ret = rknn.export_rknn(output_path)
    if ret != 0:
        print("Export rknn model failed!")
        sys.exit(ret)
    print("done, exported to {}".format(output_path))
    print("SHA-256: {}".format(sha256_of(output_path)))

    rknn.release()
