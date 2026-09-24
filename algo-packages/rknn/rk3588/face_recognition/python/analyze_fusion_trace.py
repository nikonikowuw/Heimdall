#!/usr/bin/env python3
"""时域融合标定分析器。

消费 `face_fusion_probe` 输出的 JSONL 轨迹，回答三个问题：

1. **融合到底有没有发生**：接受帧数、`fused_count` 演化、成熟握手；
2. **融合有没有把模板推向底库**：`cos(模板快照, 底库)` 随融合深度的轨迹；
3. **模板自 seed 起位移了多少**：位移角是否超过余弦分数可感阈值，还是仅方差收益。

关键语义（易错点）：sidecar 里的 `embedding` 是**融合后的模板快照**
（`plugin.rs` 的 `BestShotSidecar.embedding`），**不是**该帧自身的单帧特征。
因此本工具计算的是「模板轨迹」而不是「样本相似度矩阵」；要评估冗余门、
一致性权重等选样策略，需要包内额外透出逐样本向量（当前未实现）。

底库向量三种来源（可同时给出，分别对应生产口径与自洽口径）：
  * `--gallery-db Heimdall.db --gallery-face <face_id|latest>`：生产口径（注册 TTA 模板）；
  * `--gallery-trace gallery.jsonl`：自洽口径（底库照走流式单遍路径，与探针同域）；
  * `--gallery-vector vec.f32`：裸 512×f32 小端文件。

用法：
    python3 python/analyze_fusion_trace.py \\
        --trace trace_10fps.jsonl \\
        --gallery-db /tmp/Heimdall.db --gallery-face latest \\
        --gallery-trace gallery_single.jsonl
"""

from __future__ import annotations

import argparse
import base64
import json
import math
import sqlite3
import struct
import sys
from pathlib import Path

DIM = 512
BYTES_PER_VECTOR = DIM * 4

# 与包内 best_shot.rs 对齐的常量（仅用于判读，不改变板上行为）
MIN_FUSION_FRAME_INTERVAL = 6
REDUNDANCY_SIMILARITY = 0.85
DRIFT_REJECTION_SIMILARITY = 0.55
TOP_K_FUSED_FRAMES = 4

# 余弦分数可感阈值：位移角 2.6° 约对应 0.001 的余弦变化，对齐包内 0.08 的质量平台期量级
NO_OP_COSINE = 0.999


def decode_embedding_base64(value: str) -> list[float]:
    raw = base64.b64decode(value)
    if len(raw) != BYTES_PER_VECTOR:
        raise ValueError(f"embedding 长度非法: {len(raw)} 字节（期望 {BYTES_PER_VECTOR}）")
    return list(struct.unpack(f"<{DIM}f", raw))


def decode_vector_bytes(raw: bytes) -> list[float]:
    if len(raw) != BYTES_PER_VECTOR:
        raise ValueError(f"底库向量长度非法: {len(raw)} 字节（期望 {BYTES_PER_VECTOR}）")
    return list(struct.unpack(f"<{DIM}f", raw))


def cosine(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(y * y for y in b))
    if na <= 0.0 or nb <= 0.0:
        return 0.0
    return max(-1.0, min(1.0, dot / (na * nb)))


def angle_between(a: list[float], b: list[float]) -> float:
    """两向量夹角（度）。"""
    return math.degrees(math.acos(max(-1.0, min(1.0, cosine(a, b)))))


class Snapshot:
    """携带模板快照的接受帧。

    `embedding` 是**该帧的融合模板**（`fused_count` 个样本的质量加权平均），
    而非该帧自身的单帧特征；仅当 `fused_count == 1` 时两者等价。
    """

    __slots__ = (
        "frame_index",
        "frame_id",
        "quality",
        "fused_count",
        "template_quality",
        "mature",
        "embedding",
    )

    def __init__(self, frame_index, frame_id, quality, fused_count, template_quality, mature, embedding):
        self.frame_index = frame_index
        self.frame_id = frame_id
        self.quality = quality
        self.fused_count = fused_count
        self.template_quality = template_quality
        self.mature = mature
        self.embedding = embedding


def load_trace(path: Path):
    """返回 (snapshots, observed)：snapshots 携带模板快照，observed 为全部过门人脸帧。"""
    snapshots: list[Snapshot] = []
    observed: list[tuple[int, int, float | None]] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            record = json.loads(line)
            result = record.get("result") or {}
            faces = [
                obj["face"]
                for obj in (result.get("objects") or [])
                if isinstance(obj.get("face"), dict)
            ]
            if not faces:
                continue
            # 单人 clip：取置信度最高的那张脸；多人场景应按轨道拆分后单独分析。
            face = max(faces, key=lambda item: item.get("confidence", 0.0))
            quality = face.get("quality_score")
            frame_index = record.get("frame_index", -1)
            frame_id = record.get("frame_id", -1)
            observed.append((frame_index, frame_id, quality))
            if face.get("embedding"):
                snapshots.append(
                    Snapshot(
                        frame_index=frame_index,
                        frame_id=frame_id,
                        quality=quality if quality is not None else float("nan"),
                        fused_count=face.get("fused_count"),
                        template_quality=face.get("template_quality"),
                        mature=bool(face.get("template_mature")),
                        embedding=decode_embedding_base64(face["embedding"]),
                    )
                )
    return snapshots, observed


def load_gallery_vector(args) -> tuple[str, list[float] | None]:
    if args.gallery_vector:
        path = Path(args.gallery_vector)
        if path.suffix == ".json":
            values = json.loads(path.read_text(encoding="utf-8"))
            return f"文件 {path.name} (JSON)", [float(v) for v in values]
        return f"文件 {path.name} (f32 LE)", decode_vector_bytes(path.read_bytes())
    if args.gallery_db:
        con = sqlite3.connect(f"file:{args.gallery_db}?mode=ro", uri=True)
        cur = con.cursor()
        if args.gallery_face in (None, "latest"):
            row = cur.execute(
                "SELECT face_id, feature_vector FROM gallery_faces "
                "WHERE feature_vector IS NOT NULL ORDER BY created_at DESC LIMIT 1"
            ).fetchone()
        else:
            row = cur.execute(
                "SELECT face_id, feature_vector FROM gallery_faces WHERE face_id LIKE ? LIMIT 1",
                (f"{args.gallery_face}%",),
            ).fetchone()
        if not row:
            raise SystemExit(f"底库中未找到 face_id={args.gallery_face} 的向量")
        face_id, blob = row
        return f"DB gallery_faces.face_id={face_id}", decode_vector_bytes(blob)
    if args.gallery_trace:
        snapshots, _ = load_trace(Path(args.gallery_trace))
        if not snapshots:
            return f"trace {Path(args.gallery_trace).name}（无模板快照）", None
        return f"trace {Path(args.gallery_trace).name}（流式单遍）", snapshots[-1].embedding
    return "未提供", None


def verdict(cos_seed, cos_final, shift_cos, depth: int, snapshot_count: int) -> list[str]:
    """按实测模板轨迹给出判读结论。"""
    if snapshot_count == 0:
        return ["未产生任何模板快照 —— 身份链路未启动（见摘要中的质量门诊断）"]
    if depth <= 1:
        return [
            "融合深度始终为 1：模板 ≡ 单帧特征，本序列上不存在多帧融合。",
            "要调的是「为什么只接受了一次」：质量门（max(quality_min, fusion_min)）/ "
            f"{MIN_FUSION_FRAME_INTERVAL} 帧提取间隔 × 分析帧率 / 宿主结算窗口。",
        ]
    lines = [
        f"融合深度 {depth}：确实发生了多帧平均。",
    ]
    if shift_cos is not None:
        angle = math.degrees(math.acos(max(-1.0, min(1.0, shift_cos))))
        if shift_cos >= NO_OP_COSINE:
            lines.append(
                f"但模板自 seed 起仅位移 {angle:.2f}°（cos={shift_cos:.4f}）→ "
                "参与融合的样本与模板近乎同向，只有方差收益，对余弦分数的影响 < 0.002。"
            )
        else:
            lines.append(f"模板自 seed 起位移 {angle:.2f}°（cos={shift_cos:.4f}）→ 融合改变了模板方向。")
    if cos_seed is not None and cos_final is not None:
        delta = cos_final - cos_seed
        if delta > 0.01:
            lines.append(f"cos(模板, 库) 由 {cos_seed:+.4f} 提升到 {cos_final:+.4f}（Δ={delta:+.4f}）→ 融合把探针推向了底库。")
        elif delta < -0.01:
            lines.append(
                f"cos(模板, 库) 由 {cos_seed:+.4f} 恶化到 {cos_final:+.4f}（Δ={delta:+.4f}）→ "
                "融合把探针拖离了底库域，优先检查一致性权重与离群剔除。"
            )
        else:
            lines.append(f"cos(模板, 库) 基本不变（{cos_seed:+.4f} → {cos_final:+.4f}）→ 本序列上融合无增益。")
    return lines


def main() -> int:
    parser = argparse.ArgumentParser(description="时域融合标定分析器")
    parser.add_argument("--trace", required=True, help="face_fusion_probe 输出的 JSONL")
    parser.add_argument("--gallery-db", help="Heimdall.db 路径（生产口径底库）")
    parser.add_argument("--gallery-face", default="latest", help="底库 face_id 前缀，默认 latest")
    parser.add_argument("--gallery-trace", help="底库照单帧 trace（自洽口径）")
    parser.add_argument("--gallery-vector", help="裸 512×f32 LE 或 JSON 向量文件")
    parser.add_argument(
        "--quality-gate",
        type=float,
        default=None,
        help="提取质量门 = max(quality_min_score, fusion_min_quality_score)，用于决策重建",
    )
    args = parser.parse_args()

    snapshots, observed = load_trace(Path(args.trace))
    gallery_label, gallery = load_gallery_vector(args)

    print("=" * 72)
    print("  时域融合标定分析")
    print("=" * 72)
    print(f"  轨迹: {args.trace}")
    print(f"  过门人脸帧: {len(observed)}，携带模板快照的接受帧: {len(snapshots)}")
    print(f"  底库来源: {gallery_label}")

    if not observed:
        print("\n  轨迹中没有任何过门人脸 —— 检测/质量门未打开，身份链路未启动。")
        return 0

    qualities = [q for _, _, q in observed if q is not None]
    if qualities:
        print(
            f"  观测量: quality ∈ [{min(qualities):.3f}, {max(qualities):.3f}]，"
            f"均值 {sum(qualities) / len(qualities):.3f}"
        )

    print("\n[1] 模板快照轨迹（每个接受帧 = 一次模板重算）")
    if not snapshots:
        print("      无")
    for index, snap in enumerate(snapshots):
        gap = "-" if index == 0 else str(snap.frame_id - snapshots[index - 1].frame_id)
        template_quality = "None" if snap.template_quality is None else f"{snap.template_quality:.3f}"
        print(
            f"      #{index:<2} frame={snap.frame_index:<4} fid={snap.frame_id:<5} gap={gap:>3} "
            f"q={snap.quality:.3f} fused={snap.fused_count} tq={template_quality} "
            f"mature={'Y' if snap.mature else '-'}"
        )
    gaps = [snapshots[i].frame_id - snapshots[i - 1].frame_id for i in range(1, len(snapshots))]
    if gaps:
        print(f"      接受间隔（帧）: {gaps}（包内下限 {MIN_FUSION_FRAME_INTERVAL}）")

    print("\n[2] 未被接受的高质量帧（决策重建线索）")
    if args.quality_gate is None:
        print("      跳过（未提供 --quality-gate）")
    else:
        accepted_ids = {snap.frame_id for snap in snapshots}
        for frame_index, frame_id, quality in observed:
            if quality is None or quality < args.quality_gate or frame_id in accepted_ids:
                continue
            previous = [snap for snap in snapshots if snap.frame_id < frame_id]
            if not previous:
                reason = "尚无接受帧"
            else:
                delta = frame_id - previous[-1].frame_id
                best = max(snap.quality for snap in previous)
                if delta < MIN_FUSION_FRAME_INTERVAL:
                    reason = f"距上次提取 {delta} 帧 < {MIN_FUSION_FRAME_INTERVAL} → 提取间隔门"
                elif quality < best:
                    reason = f"q={quality:.3f} < 池内最佳 {best:.3f}（fill 策略要求 q ≥ 池内最佳）"
                else:
                    reason = "漂移拒绝 / 退避 / 池满后非 +0.08 提升"
            print(f"      frame={frame_index:<4} fid={frame_id:<5} q={quality:.3f} → {reason}")

    depth = max((snap.fused_count or 0) for snap in snapshots) if snapshots else 0
    cos_seed = cos_final = shift_cos = None
    if gallery is not None and snapshots:
        print("\n[3] 与底库的余弦（模板快照轨迹）")
        cosines = [cosine(snap.embedding, gallery) for snap in snapshots]
        for index, value in enumerate(cosines):
            print(f"      #{index:<2} fused={snapshots[index].fused_count} cos={value:+.4f}")
        cos_seed = cosines[0]
        cos_final = cosines[-1]
        shift_cos = cosine(snapshots[-1].embedding, snapshots[0].embedding)
        print(f"      seed（fused={snapshots[0].fused_count}）cos={cos_seed:+.4f}")
        print(f"      final（fused={snapshots[-1].fused_count}）cos={cos_final:+.4f}")
        print(
            f"      模板总位移: cos(seed→final)={shift_cos:+.4f} "
            f"({angle_between(snapshots[0].embedding, snapshots[-1].embedding):.2f}°)"
        )
    elif snapshots:
        print("\n[3] 与底库的余弦：跳过（未提供底库向量）")

    if len(snapshots) >= 2:
        print("\n[4] 模板轨迹区间位移（相邻快照夹角，度）")
        for index in range(1, len(snapshots)):
            print(
                f"      #{index - 1} → #{index}: "
                f"{angle_between(snapshots[index - 1].embedding, snapshots[index].embedding):.2f}° "
                f"cos={cosine(snapshots[index - 1].embedding, snapshots[index].embedding):+.4f}"
            )

    print("\n[5] 结论")
    for line in verdict(cos_seed, cos_final, shift_cos, depth, len(snapshots)):
        print(f"      - {line}")
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
