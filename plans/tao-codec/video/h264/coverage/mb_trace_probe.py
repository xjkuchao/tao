#!/usr/bin/env python3
"""
H264 宏块轨迹最小复现工具.

用途:
1. 运行 1 帧最小复现并提取 I-slice 宏块轨迹.
2. 统计提前结束位置(decoded_mbs/last_mb/cabac_bits).
3. 与历史基线轨迹对比, 定位首个分歧点.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import List, Optional


RE_MB = re.compile(r"\[H264\]\[I-slice\] mb=\((\d+), (\d+)\), mb_type=(\d+)")
RE_EARLY_END = re.compile(
    r"\[H264\]\[I-slice\] 提前结束: .*decoded_mbs=(\d+), last_mb=\((\d+), (\d+)\), cabac_bits=(\d+)/(\d+)"
)
RE_COMPARE = re.compile(
    r"Tao/FFmpeg: max_err=([-+]?\d+(?:\.\d+)?), psnr=([-+]?\d+(?:\.\d+)?)dB, 精度=([-+]?\d+(?:\.\d+)?)%"
)


@dataclass
class MbEvent:
    mb_x: int
    mb_y: int
    mb_type: int


@dataclass
class ProbeResult:
    input_path: str
    frames: int
    mb_range: str
    decoded_mbs: Optional[int]
    last_mb_x: Optional[int]
    last_mb_y: Optional[int]
    cabac_bits_used: Optional[int]
    cabac_bits_total: Optional[int]
    max_err: Optional[float]
    psnr: Optional[float]
    precision: Optional[float]
    mb_trace: List[MbEvent]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="H264 宏块轨迹最小复现诊断工具")
    parser.add_argument(
        "--input",
        default="data/1_h264.mp4",
        help="输入样本路径, 默认 data/1_h264.mp4",
    )
    parser.add_argument(
        "--frames",
        type=int,
        default=1,
        help="对比帧数, 默认 1",
    )
    parser.add_argument(
        "--mb-range",
        default="0:260",
        help="宏块调试范围(传给 TAO_H264_DEBUG_MB_RANGE), 默认 0:260",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=120,
        help="cargo test 超时秒数, 默认 120",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        help="基线 JSON 文件路径, 若提供则执行首个分歧点对比",
    )
    parser.add_argument(
        "--save-baseline",
        type=Path,
        help="将当前结果保存为基线 JSON",
    )
    return parser.parse_args()


def choose_test_target(input_path: str) -> str:
    normalized = input_path.replace("\\", "/")
    if normalized.endswith("data/1_h264.mp4"):
        return "h264::test_h264_compare_sample_1"
    if normalized.endswith("data/2_h264.mp4"):
        return "h264::test_h264_compare_sample_2"
    return "h264::test_h264_compare"


def run_probe(input_path: str, frames: int, mb_range: str, timeout: int) -> str:
    env = os.environ.copy()
    env["TAO_H264_COMPARE_INPUT"] = input_path
    env["TAO_H264_COMPARE_FRAMES"] = str(frames)
    env["TAO_H264_COMPARE_REQUIRED_PRECISION"] = "0"
    env["TAO_H264_DEBUG_MB"] = "1"
    env["TAO_H264_DEBUG_MB_RANGE"] = mb_range

    cmd = [
        "cargo",
        "test",
        "--test",
        "run_decoder",
        choose_test_target(input_path),
        "--",
        "--ignored",
        "--nocapture",
    ]
    proc = subprocess.run(
        cmd,
        env=env,
        text=True,
        capture_output=True,
        timeout=max(timeout, 1),
        check=False,
    )
    return (proc.stdout or "") + "\n" + (proc.stderr or "")


def parse_result(output: str, input_path: str, frames: int, mb_range: str) -> ProbeResult:
    mb_trace: List[MbEvent] = []
    decoded_mbs = None
    last_mb_x = None
    last_mb_y = None
    cabac_bits_used = None
    cabac_bits_total = None
    max_err = None
    psnr = None
    precision = None

    in_first_i_slice = False
    first_i_slice_done = False

    for line in output.splitlines():
        if not in_first_i_slice and "[H264][SliceHeader]" in line and "slice_type=2" in line:
            in_first_i_slice = True
            continue

        if in_first_i_slice and not first_i_slice_done:
            m = RE_MB.search(line)
            if m:
                mb_trace.append(
                    MbEvent(mb_x=int(m.group(1)), mb_y=int(m.group(2)), mb_type=int(m.group(3)))
                )
                continue

            m = RE_EARLY_END.search(line)
            if m and decoded_mbs is None:
                decoded_mbs = int(m.group(1))
                last_mb_x = int(m.group(2))
                last_mb_y = int(m.group(3))
                cabac_bits_used = int(m.group(4))
                cabac_bits_total = int(m.group(5))
                continue

            if "[H264][I-slice] 完成:" in line:
                first_i_slice_done = True
                continue

        m = RE_COMPARE.search(line)
        if m and precision is None:
            max_err = float(m.group(1))
            psnr = float(m.group(2))
            precision = float(m.group(3))

    return ProbeResult(
        input_path=input_path,
        frames=frames,
        mb_range=mb_range,
        decoded_mbs=decoded_mbs,
        last_mb_x=last_mb_x,
        last_mb_y=last_mb_y,
        cabac_bits_used=cabac_bits_used,
        cabac_bits_total=cabac_bits_total,
        max_err=max_err,
        psnr=psnr,
        precision=precision,
        mb_trace=mb_trace,
    )


def load_baseline(path: Path) -> ProbeResult:
    data = json.loads(path.read_text(encoding="utf-8"))
    trace = [
        MbEvent(mb_x=int(item["mb_x"]), mb_y=int(item["mb_y"]), mb_type=int(item["mb_type"]))
        for item in data.get("mb_trace", [])
    ]
    return ProbeResult(
        input_path=data.get("input_path", ""),
        frames=int(data.get("frames", 0)),
        mb_range=data.get("mb_range", ""),
        decoded_mbs=data.get("decoded_mbs"),
        last_mb_x=data.get("last_mb_x"),
        last_mb_y=data.get("last_mb_y"),
        cabac_bits_used=data.get("cabac_bits_used"),
        cabac_bits_total=data.get("cabac_bits_total"),
        max_err=data.get("max_err"),
        psnr=data.get("psnr"),
        precision=data.get("precision"),
        mb_trace=trace,
    )


def compare_trace(cur: ProbeResult, base: ProbeResult) -> None:
    n = min(len(cur.mb_trace), len(base.mb_trace))
    first_diff = None
    for i in range(n):
        a = cur.mb_trace[i]
        b = base.mb_trace[i]
        if (a.mb_x, a.mb_y, a.mb_type) != (b.mb_x, b.mb_y, b.mb_type):
            first_diff = (i, a, b)
            break

    if first_diff is not None:
        i, a, b = first_diff
        print(
            f"轨迹分歧: idx={i}, 当前=({a.mb_x},{a.mb_y},type={a.mb_type}), "
            f"基线=({b.mb_x},{b.mb_y},type={b.mb_type})"
        )
    elif len(cur.mb_trace) != len(base.mb_trace):
        print(f"轨迹长度不同: 当前={len(cur.mb_trace)}, 基线={len(base.mb_trace)}")
    else:
        print("轨迹完全一致")


def save_baseline(path: Path, result: ProbeResult) -> None:
    payload = asdict(result)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def print_summary(result: ProbeResult) -> None:
    print(f"输入: {result.input_path}")
    print(f"帧数: {result.frames}, MB范围: {result.mb_range}")
    print(f"采样宏块数: {len(result.mb_trace)}")
    if result.decoded_mbs is not None:
        print(
            f"提前结束: decoded_mbs={result.decoded_mbs}, "
            f"last_mb=({result.last_mb_x},{result.last_mb_y}), "
            f"cabac_bits={result.cabac_bits_used}/{result.cabac_bits_total}"
        )
    if result.precision is not None:
        print(
            f"精度: {result.precision:.6f}%, psnr={result.psnr:.4f}dB, max_err={result.max_err:.1f}"
        )


def main() -> None:
    args = parse_args()
    output = run_probe(args.input, args.frames, args.mb_range, args.timeout)
    result = parse_result(output, args.input, args.frames, args.mb_range)
    print_summary(result)

    if args.baseline:
        baseline = load_baseline(args.baseline)
        compare_trace(result, baseline)

    if args.save_baseline:
        save_baseline(args.save_baseline, result)
        print(f"已保存基线: {args.save_baseline}")


if __name__ == "__main__":
    main()
