#!/usr/bin/env python3
"""
YOLOv8 烟火检测模型导出与量化工具 (Rockchip RK3568 RKNN)

模型来源:
  /home/nikoniko/work/tentcoo/rknn_model_zoo/examples/fire-detections-yolov8
  原始 ONNX: best.onnx (640x384 16:9, 9-tensor 多分支优化输出)

用法:
  python3 convert.py <onnx_model_path> [platform] [dtype] [output_rknn_path]

示例:
  python3 convert.py ../model/best.onnx rk3568 i8 ../model/best_hybrid.rknn
"""

import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
LOCAL_DATASET = os.path.join(SCRIPT_DIR, "fire-dataset/dataset.txt")
FALLBACK_DATASET = os.path.join(
    SCRIPT_DIR, "../../../datasets/fire-dataset/dataset.txt"
)

if os.path.exists(LOCAL_DATASET):
    DATASET_PATH = LOCAL_DATASET
elif os.path.exists(FALLBACK_DATASET):
    DATASET_PATH = FALLBACK_DATASET
else:
    DATASET_PATH = os.path.join(SCRIPT_DIR, "fire-dataset/dataset.txt")

DEFAULT_RKNN_PATH = os.path.join(SCRIPT_DIR, "../model/best_hybrid.rknn")
DEFAULT_QUANT = True


def parse_arg():
    if len(sys.argv) < 2:
        print(f"Usage: python3 {sys.argv[0]} onnx_model_path [platform] [dtype] [output_rknn_path]")
        print("       platform: [rk3562, rk3566, rk3568, rk3576, rk3588, rv1126b, rv1109, rv1126, rk1808] (default: rk3568)")
        print("       dtype: [i8, fp] (default: i8)")
        print("       output_rknn_path: default ../model/best_hybrid.rknn")
        exit(1)

    model_path = sys.argv[1]
    platform = sys.argv[2] if len(sys.argv) > 2 else "rk3568"

    do_quant = DEFAULT_QUANT
    if len(sys.argv) > 3:
        model_type = sys.argv[3]
        if model_type not in ["i8", "u8", "fp"]:
            print(f"ERROR: Invalid model type: {model_type}")
            exit(1)
        elif model_type in ["i8", "u8"]:
            do_quant = True
        else:
            do_quant = False

    output_path = sys.argv[4] if len(sys.argv) > 4 else DEFAULT_RKNN_PATH

    return model_path, platform, do_quant, output_path


def ensure_dataset_txt(dataset_txt_path):
    """确保 dataset.txt 内的图片路径存在，若路径无效则动态重写为绝对路径"""
    dataset_dir = os.path.dirname(dataset_txt_path)
    if not os.path.exists(dataset_dir):
        return

    # 扫描 fire-dataset 目录下的图片
    valid_exts = {".png", ".jpg", ".jpeg", ".bmp"}
    images = [
        os.path.join(dataset_dir, f)
        for f in sorted(os.listdir(dataset_dir))
        if os.path.splitext(f)[1].lower() in valid_exts
    ]

    if images:
        with open(dataset_txt_path, "w") as f:
            for img_path in images:
                f.write(img_path + "\n")
        print(f"--> 已动态刷新量化校准数据集: {dataset_txt_path} ({len(images)} 张图片)")


if __name__ == "__main__":
    from rknn.api import RKNN

    model_path, platform, do_quant, output_path = parse_arg()

    # 自动校验与生成本地数据集列表
    ensure_dataset_txt(DATASET_PATH)

    # 创建 RKNN 转换对象
    rknn = RKNN(verbose=False)

    # 预处理与量化参数配置
    print("--> Config model")
    rknn.config(
        mean_values=[[0, 0, 0]],
        std_values=[[255, 255, 255]],
        target_platform=platform,
        # 混合精度余弦相似度阈值 (保留对敏感层的高精度表示)
        auto_hybrid_cos_thresh=0.98,
    )
    print("done")

    # 加载 ONNX 模型
    print(f"--> Loading model: {model_path}")
    ret = rknn.load_onnx(model=model_path)
    if ret != 0:
        print("Load model failed!")
        exit(ret)
    print("done")

    # 构建模型并执行量化
    print(f"--> Building model (quantization={do_quant}, dataset={DATASET_PATH})")
    ret = rknn.build(
        do_quantization=do_quant,
        dataset=DATASET_PATH if do_quant else None,
        auto_hybrid=True if do_quant else False,
    )
    if ret != 0:
        print("Build model failed!")
        exit(ret)
    print("done")

    # 导出 RKNN 模型
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    print(f"--> Export rknn model: {output_path}")
    ret = rknn.export_rknn(output_path)
    if ret != 0:
        print("Export rknn model failed!")
        exit(ret)
    print("done")

    # 释放资源
    rknn.release()
