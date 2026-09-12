#!/usr/bin/env python3
"""
Convert yolov8n-face ONNX model to RKNN format.

Usage:
    python convert.py <onnx_model_path> <platform> [dtype] [output_rknn_path]

Examples:
    # Convert 640x384 model
    python convert.py ../model/yolov8n-face-640x384.onnx rk3576 i8 ../model/yolov8n-face-640x384_i8.rknn

    # Convert 640x640 model
    python convert.py ../model/yolov8n-face-640x640.onnx rk3576 i8 ../model/yolov8n-face-640x640_i8.rknn
"""

import os
import sys
from pathlib import Path

from rknn.api import RKNN

# Resolve dataset path relative to this script's location
SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_DIR = SCRIPT_DIR.parent
DATASET_PATH = str(PROJECT_DIR.parent.parent / "datasets" / "COCO" / "coco_subset_20.txt")
DEFAULT_QUANT = True


def parse_arg():
    if len(sys.argv) < 3:
        print(
            "Usage: python3 {} onnx_model_path [platform] [dtype(optional)] [output_rknn_path(optional)]".format(
                sys.argv[0]
            )
        )
        print(
            "       platform choose from [rk3562, rk3566, rk3568, rk3576, rk3588, rv1126b, rv1109, rv1126, rk1808]"
        )
        print(
            "       dtype choose from [i8, fp] for [rk3562, rk3566, rk3568, rk3576, rk3588, rv1126b]"
        )
        print("       dtype choose from [u8, fp] for [rv1109, rv1126, rk1808]")
        exit(1)

    model_path = sys.argv[1]
    platform = sys.argv[2]

    do_quant = DEFAULT_QUANT
    if len(sys.argv) > 3:
        model_type = sys.argv[3]
        if model_type not in ["i8", "u8", "fp"]:
            print("ERROR: Invalid model type: {}".format(model_type))
            exit(1)
        elif model_type in ["i8", "u8"]:
            do_quant = True
        else:
            do_quant = False

    # Default output path: same name as input but with .rknn extension
    if len(sys.argv) > 4:
        output_path = sys.argv[4]
    else:
        # Replace .onnx with .rknn, keep the rest of the name
        output_path = str(Path(model_path).with_suffix(".rknn"))

    return model_path, platform, do_quant, output_path


if __name__ == "__main__":
    model_path, platform, do_quant, output_path = parse_arg()

    # Resolve paths relative to current working directory
    model_path = str(Path.cwd() / model_path) if not Path(model_path).is_absolute() else model_path
    output_path = str(Path.cwd() / output_path) if not Path(output_path).is_absolute() else output_path

    # Create RKNN object
    rknn = RKNN(verbose=False)

    # Pre-process config
    # yolov8 uses RGB input normalized to [0, 1] (mean=0, std=255)
    print("--> Config model")
    rknn.config(
        mean_values=[[0, 0, 0]], std_values=[[255, 255, 255]], target_platform=platform
    )
    print("done")

    # Load model
    print("--> Loading model")
    ret = rknn.load_onnx(model=model_path)
    if ret != 0:
        print("Load model failed!")
        exit(ret)
    print("done")

    # Build model
    print("--> Building model")
    print("   dataset: {}".format(DATASET_PATH))
    ret = rknn.build(do_quantization=do_quant, dataset=DATASET_PATH)
    if ret != 0:
        print("Build model failed!")
        exit(ret)
    print("done")

    # Export rknn model
    print("--> Export rknn model")
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    ret = rknn.export_rknn(output_path)
    if ret != 0:
        print("Export rknn model failed!")
        exit(ret)
    print("done, exported to {}".format(output_path))

    # Release
    rknn.release()
