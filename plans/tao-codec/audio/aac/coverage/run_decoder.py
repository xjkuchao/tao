#!/usr/bin/env python3
import argparse
import os
import re
import subprocess
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from multiprocessing import cpu_count
from pathlib import Path

REPORT_PATH = Path("plans/tao-codec/audio/aac/coverage/report.md")
NO_AUDIO_SAMPLE_INDEXES = {
    20,
    49,
    50,
    55,
    57,
    60,
    63,
    67,
    68,
    79,
    80,
    86,
    87,
    92,
    93,
    94,
    95,
    96,
    97,
    98,
    99,
    107,
}
SBR_SAMPLE_RATE_MISMATCH_INDEXES = {5, 6, 7, 8, 15, 26, 27, 28, 29}
INVALID_ADTS_SAMPLE_INDEXES = {36, 102, 111}
NON_AAC_STREAM_INDEXES = {54}
INVALID_CONTAINER_INDEXES = {33, 110}
CORRUPTED_STREAM_INDEXES = {37}
# 按用户要求禁用默认跳过策略, 全量样本统一进入真实回归.
SKIPPED_SAMPLE_INDEXES = set()
SKIPPED_SAMPLE_REASONS = {}

HEADER_PREFIX = "| 序号 |"
SEP_PREFIX = "| --- |"

LINE_RE = re.compile(
    r"Tao对比样本=(\d+), Tao=(\d+), FFmpeg=(\d+), (?:lag=[-+]?\d+, )?Tao/FFmpeg: "
    r"max_err=([-+]?[0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?), "
    r"psnr=([A-Za-z]+|[-+]?[0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?)dB, "
    r"精度=([-+]?[0-9]*\.?[0-9]+)%"
)


def parse_args():
    parser = argparse.ArgumentParser(
        description="AAC 解码器样本批量对比工具, 从项目根目录运行.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  # 默认断点续测(只处理尚未测试的记录)
  python plans/tao-codec/audio/aac/coverage/run_decoder.py

  # 重新测试所有精度不为 100%% 的记录(含失败)
  python plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-imprecise

  # 只重新测试失败的记录
  python plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-failed

  # 重新测试全部记录
  python plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-all

  # 只测试指定序号(可多个)
  python plans/tao-codec/audio/aac/coverage/run_decoder.py --index 3 5 8

  # 指定并行数量
  python plans/tao-codec/audio/aac/coverage/run_decoder.py --jobs 4
        """,
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--retest-all", action="store_true", help="重新测试所有记录")
    group.add_argument(
        "--retest-failed", action="store_true", help="重新测试状态为失败的记录"
    )
    group.add_argument(
        "--retest-imprecise",
        action="store_true",
        help="重新测试精度不为 100%% 的记录(含失败)",
    )
    parser.add_argument(
        "--index",
        type=int,
        nargs="+",
        metavar="N",
        help="只测试指定序号的记录(可多个, 与上述参数可组合)",
    )
    parser.add_argument(
        "--jobs",
        "-j",
        type=int,
        default=cpu_count(),
        metavar="N",
        help=f"并行处理数量(默认: CPU 核心数, 当前 {cpu_count()})",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=60,
        metavar="SEC",
        help="单个样本测试超时秒数(默认: 60)",
    )
    parser.add_argument(
        "--include-skipped",
        action="store_true",
        help="包含默认跳过样本(用于手动复测, 默认不包含)",
    )
    return parser.parse_args()


def split_row(line):
    parts = [p.strip() for p in line.strip().split("|")]
    if len(parts) < 3:
        return []
    return parts[1:-1]


def load_report():
    if not REPORT_PATH.exists():
        raise RuntimeError("报告文件不存在, 请先生成报告模板.")
    lines = REPORT_PATH.read_text(encoding="utf-8").splitlines()

    header_idx = None
    for i, line in enumerate(lines):
        if line.startswith(HEADER_PREFIX):
            header_idx = i
            break
    if header_idx is None or header_idx + 1 >= len(lines):
        raise RuntimeError("报告表头缺失, 无法继续.")

    header = split_row(lines[header_idx])
    sep = lines[header_idx + 1]
    if not sep.startswith(SEP_PREFIX):
        raise RuntimeError("报告分隔行缺失, 无法继续.")

    data_start = header_idx + 2
    rows = []
    for line in lines[data_start:]:
        if not line.startswith("|"):
            break
        cols = split_row(line)
        if cols:
            rows.append(cols)

    return lines, header_idx, header, sep, rows


def write_report(lines, header_idx, sep, rows):
    out = []
    out.extend(lines[:header_idx])
    out.append(lines[header_idx])
    out.append(sep)
    for cols in rows:
        out.append("| " + " | ".join(cols) + " |")
    out.extend(lines[header_idx + 2 + len(rows) :])
    REPORT_PATH.write_text("\n".join(out) + "\n", encoding="utf-8")


def run_compare(url, timeout_sec):
    env = os.environ.copy()
    env["TAO_AAC_COMPARE_INPUT"] = url
    cmd = [
        "cargo",
        "test",
        "--test",
        "run_decoder",
        "aac::",
        "--",
        "--nocapture",
        "--ignored",
    ]
    try:
        proc = subprocess.run(
            cmd,
            text=True,
            capture_output=True,
            env=env,
            timeout=max(1, timeout_sec),
        )
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        output = stdout + "\n" + stderr
        output += f"\n单样本测试超时: {timeout_sec}s"
        return 124, output

    output = (proc.stdout or "") + "\n" + (proc.stderr or "")
    return proc.returncode, output


def parse_metrics(output):
    for line in output.splitlines():
        if "Tao对比样本=" in line and "Tao/FFmpeg:" in line:
            m = LINE_RE.search(line)
            if not m:
                continue
            tao_samples = int(m.group(2))
            ff_samples = int(m.group(3))
            return {
                "tao_samples": tao_samples,
                "ff_samples": ff_samples,
                "sample_diff": tao_samples - ff_samples,
                "max_err": m.group(4),
                "psnr": m.group(5),
                "precision": f"{float(m.group(6)):.2f}",
            }
    return None


def extract_failure_reason(output):
    lines = [ln.strip() for ln in output.splitlines() if ln.strip()]
    if not lines:
        return "无输出"

    keywords = (
        "AAC 对比",
        "缺少对比输入参数",
        "未找到可解码音频流",
        "ffmpeg 解码失败",
        "打开输入失败",
        "单样本测试超时",
        "解析失败",
    )
    for ln in reversed(lines):
        if any(k in ln for k in keywords):
            return ln.replace("|", "/")

    tail = lines[-3:]
    return " / ".join(ln.replace("|", "/") for ln in tail)


def should_skip(row, col_map, args, idx):
    if not args.include_skipped and idx in SKIPPED_SAMPLE_INDEXES:
        return True

    if args.index and idx not in args.index:
        return True
    if args.index and idx in args.index:
        return False

    status = row[col_map["状态"]]
    precision = row[col_map["精度(%)"]]

    def is_full_precision(value):
        try:
            return float(value) >= 99.9 - 1e-9
        except (TypeError, ValueError):
            return False

    if status == "跳过" and not args.include_skipped:
        return True

    if args.retest_all:
        return False

    if args.retest_failed:
        return status != "失败"

    if args.retest_imprecise:
        if status == "失败":
            return False
        if status == "成功":
            return is_full_precision(precision)
        return True

    return status in ("成功", "失败", "跳过")


def apply_default_skip_rows(rows, col_map, args):
    if args.include_skipped:
        return False

    changed = False
    for idx, row in enumerate(rows, 1):
        if idx not in SKIPPED_SAMPLE_INDEXES:
            continue
        status = row[col_map["状态"]]
        reason = SKIPPED_SAMPLE_REASONS.get(idx, "按规则跳过")
        if status != "跳过" or row[col_map["失败原因"]] != reason:
            row[col_map["状态"]] = "跳过"
            row[col_map["失败原因"]] = reason
            row[col_map["Tao样本数"]] = ""
            row[col_map["FFmpeg样本数"]] = ""
            row[col_map["样本数差异"]] = ""
            row[col_map["max_err"]] = ""
            row[col_map["psnr(dB)"]] = ""
            row[col_map["精度(%)"]] = ""
            row[col_map["备注"]] = "已跳过"
            changed = True
    return changed


def main():
    args = parse_args()
    lines, header_idx, header, sep, rows = load_report()

    col_map = {name: idx for idx, name in enumerate(header)}
    required = [
        "序号",
        "URL",
        "状态",
        "失败原因",
        "Tao样本数",
        "FFmpeg样本数",
        "样本数差异",
        "max_err",
        "psnr(dB)",
        "精度(%)",
        "备注",
    ]
    for name in required:
        if name not in col_map:
            raise RuntimeError(f"报告表缺少列: {name}")

    if apply_default_skip_rows(rows, col_map, args):
        write_report(lines, header_idx, sep, rows)

    total = len(rows)
    pending = [
        (idx, row)
        for idx, row in enumerate(rows, 1)
        if not should_skip(row, col_map, args, idx)
    ]

    if not pending:
        print("没有需要处理的记录.")
        return

    jobs = max(1, args.jobs)
    print(
        f"共 {len(pending)} 条记录待处理, 并行数: {jobs}, 单样本超时: {args.timeout}s",
        flush=True,
    )

    lock = threading.Lock()

    def process(idx, row):
        url = row[col_map["URL"]]
        print(f"开始处理 {idx}/{total}: {url}", flush=True)
        code, output = run_compare(url, args.timeout)
        metrics = parse_metrics(output)

        if metrics is not None:
            row[col_map["状态"]] = "成功"
            row[col_map["失败原因"]] = ""
            row[col_map["Tao样本数"]] = str(metrics["tao_samples"])
            row[col_map["FFmpeg样本数"]] = str(metrics["ff_samples"])
            row[col_map["样本数差异"]] = str(metrics["sample_diff"])
            row[col_map["max_err"]] = metrics["max_err"]
            row[col_map["psnr(dB)"]] = metrics["psnr"]
            row[col_map["精度(%)"]] = metrics["precision"]
            if code != 0:
                row[col_map["备注"]] = "严格阈值未通过"
            else:
                row[col_map["备注"]] = ""
        else:
            row[col_map["状态"]] = "失败"
            row[col_map["失败原因"]] = extract_failure_reason(output)
            row[col_map["Tao样本数"]] = ""
            row[col_map["FFmpeg样本数"]] = ""
            row[col_map["样本数差异"]] = ""
            row[col_map["max_err"]] = ""
            row[col_map["psnr(dB)"]] = ""
            row[col_map["精度(%)"]] = ""
            row[col_map["备注"]] = ""

        with lock:
            write_report(lines, header_idx, sep, rows)
            print(f"已记录 {idx}/{total}: {row[col_map['状态']]}", flush=True)

    with ThreadPoolExecutor(max_workers=jobs) as executor:
        futures = {executor.submit(process, idx, row): idx for idx, row in pending}
        for future in as_completed(futures):
            future.result()

    print("处理完成.", flush=True)


if __name__ == "__main__":
    main()
