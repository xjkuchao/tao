#!/usr/bin/env python3
"""AAC 覆盖样本下载工具.

从 plans/tao-codec/audio/aac/coverage/report.md 读取目标 URL,
下载到 data/aac_samples 分层缓存目录, 并输出 URL->本地路径索引.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import unquote, urlparse

REPORT_PATH = Path("plans/tao-codec/audio/aac/coverage/report.md")
OUTPUT_MD_PATH = Path("plans/tao-codec/audio/aac/coverage/targets_local_index.md")
OUTPUT_JSON_PATH = Path("plans/tao-codec/audio/aac/coverage/targets_local_index.json")
LOCAL_ROOT = Path("data/aac_samples")

HEADER_PREFIX = "| 序号 |"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="下载 AAC 覆盖样本到 data 目录并生成索引.",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=REPORT_PATH,
        metavar="PATH",
        help=f"报告路径(默认: {REPORT_PATH})",
    )
    parser.add_argument(
        "--local-root",
        type=Path,
        default=LOCAL_ROOT,
        metavar="PATH",
        help=f"本地下载根目录(默认: {LOCAL_ROOT})",
    )
    parser.add_argument(
        "--output-md",
        type=Path,
        default=OUTPUT_MD_PATH,
        metavar="PATH",
        help=f"Markdown 索引输出路径(默认: {OUTPUT_MD_PATH})",
    )
    parser.add_argument(
        "--output-json",
        type=Path,
        default=OUTPUT_JSON_PATH,
        metavar="PATH",
        help=f"JSON 索引输出路径(默认: {OUTPUT_JSON_PATH})",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=4,
        metavar="N",
        help="并发下载数量(默认: 4)",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=30,
        metavar="SEC",
        help="连接超时秒数(默认: 30)",
    )
    parser.add_argument(
        "--retries",
        type=int,
        default=3,
        metavar="N",
        help="失败重试次数(默认: 3)",
    )
    parser.add_argument(
        "--index",
        type=int,
        nargs="+",
        metavar="N",
        help="仅下载指定序号(可多个)",
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="覆盖已存在文件(默认: 命中缓存即跳过)",
    )
    return parser.parse_args()


def split_row(line: str) -> list[str]:
    parts = [p.strip() for p in line.strip().split("|")]
    if len(parts) < 3:
        return []
    return parts[1:-1]


def read_report_rows(report_path: Path) -> list[dict[str, str]]:
    if not report_path.exists():
        raise RuntimeError(f"报告不存在: {report_path}")

    lines = report_path.read_text(encoding="utf-8").splitlines()
    header_idx = None
    for i, line in enumerate(lines):
        if line.startswith(HEADER_PREFIX):
            header_idx = i
            break
    if header_idx is None or header_idx + 1 >= len(lines):
        raise RuntimeError("报告格式错误: 缺少表头")

    header = split_row(lines[header_idx])
    col_map = {name: idx for idx, name in enumerate(header)}
    if "序号" not in col_map or "URL" not in col_map:
        raise RuntimeError("报告格式错误: 缺少 '序号' 或 'URL' 列")

    rows = []
    for line in lines[header_idx + 2 :]:
        if not line.startswith("|"):
            break
        cols = split_row(line)
        if not cols:
            continue
        idx_text = cols[col_map["序号"]]
        url = cols[col_map["URL"]]
        if not idx_text.isdigit() or not url:
            continue
        rows.append({"index": int(idx_text), "url": url})
    return rows


def sanitize_segment(segment: str) -> str:
    cleaned = unquote(segment).strip()
    if not cleaned:
        return "_"
    cleaned = cleaned.replace("\\", "_")
    cleaned = cleaned.replace("/", "_")
    cleaned = cleaned.replace("\x00", "_")
    cleaned = cleaned.replace("\n", "_")
    cleaned = cleaned.replace("\r", "_")
    if cleaned in (".", ".."):
        return "_"
    return cleaned


def local_path_from_url(url: str, local_root: Path) -> Path:
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https") or not parsed.netloc:
        raise RuntimeError(f"不支持的 URL: {url}")

    parts = [sanitize_segment(parsed.netloc)]
    for raw in parsed.path.split("/"):
        if not raw:
            continue
        parts.append(sanitize_segment(raw))

    if len(parts) == 1:
        parts.append("index.bin")
    if parsed.path.endswith("/"):
        parts.append("index.bin")

    target = local_root.joinpath(*parts)
    if parsed.query:
        digest = hashlib.sha1(parsed.query.encode("utf-8")).hexdigest()[:8]
        target = target.with_name(f"{target.name}__q_{digest}")
    return target


def download_one(url: str, local_path: Path, timeout: int, retries: int, overwrite: bool) -> dict:
    local_path.parent.mkdir(parents=True, exist_ok=True)

    if local_path.exists() and local_path.is_file() and local_path.stat().st_size > 0 and not overwrite:
        return {
            "status": "cached",
            "size_bytes": local_path.stat().st_size,
            "message": "命中本地缓存",
        }

    if overwrite and local_path.exists():
        local_path.unlink()

    cmd = [
        "curl",
        "-L",
        "--fail",
        "--silent",
        "--show-error",
        "--retry",
        str(max(0, retries)),
        "--retry-delay",
        "1",
        "--connect-timeout",
        str(max(1, timeout)),
        "--continue-at",
        "-",
        "-o",
        str(local_path),
        url,
    ]

    proc = subprocess.run(cmd, text=True, capture_output=True)
    if proc.returncode != 0:
        message = (proc.stderr or proc.stdout or "curl 下载失败").strip().replace("\n", " / ")
        if local_path.exists() and local_path.stat().st_size == 0:
            local_path.unlink(missing_ok=True)
        return {"status": "failed", "size_bytes": 0, "message": message}

    size = local_path.stat().st_size if local_path.exists() else 0
    if size <= 0:
        return {"status": "failed", "size_bytes": 0, "message": "下载完成但文件为空"}

    return {"status": "downloaded", "size_bytes": size, "message": "下载成功"}


def write_json_index(path: Path, report: Path, local_root: Path, entries: list[dict]) -> None:
    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "report_path": str(report),
        "local_root": str(local_root),
        "entries": entries,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_md_index(path: Path, report: Path, local_root: Path, entries: list[dict]) -> None:
    success = sum(1 for e in entries if e["download_status"] in ("downloaded", "cached"))
    failed = sum(1 for e in entries if e["download_status"] == "failed")

    lines = [
        "# AAC 覆盖样本本地索引",
        "",
        f"- 生成时间(UTC): {datetime.now(timezone.utc).isoformat()}",
        f"- 来源报告: `{report}`",
        f"- 本地目录: `{local_root}`",
        f"- 总条目: {len(entries)}",
        f"- 可用(下载成功+缓存命中): {success}",
        f"- 失败: {failed}",
        "",
        "| 序号 | URL | 本地路径 | 状态 | 大小(bytes) | 备注 |",
        "| --- | --- | --- | --- | --- | --- |",
    ]

    for e in entries:
        lines.append(
            "| {index} | {url} | {local_path} | {download_status} | {size_bytes} | {message} |".format(
                **e
            )
        )

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    args = parse_args()
    rows = read_report_rows(args.report)

    selected = []
    target_indexes = set(args.index) if args.index else None
    for row in rows:
        if target_indexes is not None and row["index"] not in target_indexes:
            continue
        selected.append(row)

    if not selected:
        print("没有待下载的样本.")
        return

    lock = threading.Lock()
    results: list[dict] = []

    print(
        f"开始下载 {len(selected)} 条样本, 并发={max(1, args.jobs)}, 超时={args.timeout}s, 重试={args.retries}",
        flush=True,
    )

    def worker(row: dict) -> dict:
        idx = row["index"]
        url = row["url"]
        local_path = local_path_from_url(url, args.local_root)
        print(f"[{idx}] 开始: {url}", flush=True)
        outcome = download_one(url, local_path, args.timeout, args.retries, args.overwrite)
        entry = {
            "index": idx,
            "url": url,
            "local_path": str(local_path),
            "download_status": outcome["status"],
            "size_bytes": outcome["size_bytes"],
            "message": outcome["message"].replace("|", "/"),
        }
        print(f"[{idx}] 结束: {entry['download_status']} ({entry['size_bytes']} bytes)", flush=True)
        return entry

    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as executor:
        futures = [executor.submit(worker, row) for row in selected]
        for future in as_completed(futures):
            entry = future.result()
            with lock:
                results.append(entry)

    results.sort(key=lambda x: x["index"])
    write_json_index(args.output_json, args.report, args.local_root, results)
    write_md_index(args.output_md, args.report, args.local_root, results)

    success = sum(1 for e in results if e["download_status"] in ("downloaded", "cached"))
    failed = sum(1 for e in results if e["download_status"] == "failed")
    print(f"下载完成: 总计 {len(results)}, 可用 {success}, 失败 {failed}", flush=True)
    print(f"JSON 索引: {args.output_json}", flush=True)
    print(f"MD 索引: {args.output_md}", flush=True)


if __name__ == "__main__":
    main()
