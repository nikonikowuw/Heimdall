#!/usr/bin/env python3
"""Convert EdgeFace ONNX model to RKNN format."""

import os
import sys
import types
from pathlib import Path

# Compatibility shim: newer onnx versions removed onnx.mapping.TENSOR_TYPE_TO_NP_TYPE
import onnx
if not hasattr(onnx, "mapping") or not hasattr(getattr(onnx, "mapping", None), "TENSOR_TYPE_TO_NP_TYPE"):
    mapping_module = types.ModuleType("onnx.mapping")
    if hasattr(onnx, "helper") and hasattr(onnx.helper, "tensor_dtype_to_np_dtype"):
        tensor_map = {}
        for attr in dir(onnx.TensorProto):
            if attr.isupper() and not attr.startswith("_"):
                val = getattr(onnx.TensorProto, attr)
                if isinstance(val, int):
                    try:
                        tensor_map[val] = onnx.helper.tensor_dtype_to_np_dtype(val)
                    except Exception:
                        pass
        mapping_module.TENSOR_TYPE_TO_NP_TYPE = tensor_map
        mapping_module.NP_TYPE_TO_TENSOR_TYPE = {
            v: k for k, v in tensor_map.items() if hasattr(v, "type")
        }
    onnx.mapping = mapping_module

from rknn.api import RKNN

SCRIPT_DIR = Path(__file__).resolve().parent
MODEL_DIR = SCRIPT_DIR.parent / "model"
DEFAULT_RKNN_PATH = str(MODEL_DIR / "edgeface_xs_gamma_06.rknn")
DEFAULT_DATASET_PATH = str(MODEL_DIR / "dataset.txt")

# EdgeFace expects normalized inputs: (x/255.0 - 0.5) / 0.5 = (x - 127.5) / 127.5
MEAN_VALUES = [[127.5, 127.5, 127.5]]
STD_VALUES = [[127.5, 127.5, 127.5]]


def parse_args():
    if len(sys.argv) < 3:
        print("Usage: python3 {} onnx_model_path [platform] [dtype(optional)] [output_rknn_path(optional)]".format(sys.argv[0]))
        print("       platform choose from [rk3562, rk3566, rk3568, rk3576, rk3588, rv1109, rv1126, rv1106, rv1103, rk1808]")
        print("       dtype choose from    [i8, fp] for [rk3562, rk3566, rk3568, rk3576, rk3588]")
        print("       dtype choose from    [u8, fp] for [rv1109, rv1126, rk1808, rv1106, rv1103]")
        sys.exit(1)

    model_path = sys.argv[1]
    platform = sys.argv[2]

    do_quant = False
    if len(sys.argv) > 3:
        model_type = sys.argv[3]
        if model_type not in ["i8", "u8", "fp"]:
            print(f"ERROR: Invalid model type: {model_type}")
            sys.exit(1)
        elif model_type in ["i8", "u8"]:
            do_quant = True
        else:
            do_quant = False

    if len(sys.argv) > 4:
        output_path = sys.argv[4]
    else:
        output_path = DEFAULT_RKNN_PATH

    return model_path, platform, do_quant, output_path


def main():
    model_path, platform, do_quant, output_path = parse_args()

    # Create RKNN object
    rknn = RKNN(verbose=False)

    # Pre-process config
    print("--> Config model")
    rknn.config(mean_values=MEAN_VALUES, std_values=STD_VALUES, target_platform=platform)
    print("done")

    # Load model
    print("--> Loading model")
    ret = rknn.load_onnx(model=model_path)
    if ret != 0:
        print("Load model failed!")
        sys.exit(ret)
    print("done")

    # Build model
    print("--> Building model")
    dataset = DEFAULT_DATASET_PATH if do_quant else None
    ret = rknn.build(do_quantization=do_quant, dataset=dataset)
    if ret != 0:
        print("Build model failed!")
        sys.exit(ret)
    print("done")

    # Export rknn model
    print("--> Export rknn model")
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    ret = rknn.export_rknn(output_path)
    if ret != 0:
        print("Export rknn model failed!")
        sys.exit(ret)
    print(f"done, exported to {output_path}")

    rknn.release()


if __name__ == "__main__":
    main()
