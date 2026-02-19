#!/usr/bin/env python3
import os
import re
import subprocess
from pathlib import Path

LIST_PATH = Path('plans/tao-codec_mp3_samples_urls.txt')
REPORT_PATH = Path('plans/tao-codec_mp3_samples_report.md')

HEADER_PREFIX = '| 序号 |'
SEP_PREFIX = '| --- |'

LINE_RE = re.compile(
    r'Tao对比样本=(\d+), Tao=(\d+), FFmpeg=(\d+), Tao/FFmpeg: max_err=([0-9.]+), psnr=([0-9.]+)dB, 精度=([0-9.]+)%'
)


def read_urls():
    if not LIST_PATH.exists():
        raise RuntimeError('URL清单不存在, 请先生成清单.')
    return [line.strip() for line in LIST_PATH.read_text(encoding='utf-8').splitlines() if line.strip()]


def split_row(line):
    parts = [p.strip() for p in line.strip().split('|')]
    if len(parts) < 3:
        return []
    return parts[1:-1]


def load_report():
    if not REPORT_PATH.exists():
        raise RuntimeError('报告文件不存在, 请先生成报告模板.')
    lines = REPORT_PATH.read_text(encoding='utf-8').splitlines()

    header_idx = None
    for i, line in enumerate(lines):
        if line.startswith(HEADER_PREFIX):
            header_idx = i
            break
    if header_idx is None or header_idx + 1 >= len(lines):
        raise RuntimeError('报告表头缺失, 无法继续.')

    header = split_row(lines[header_idx])
    sep = lines[header_idx + 1]
    data_start = header_idx + 2

    rows = []
    for line in lines[data_start:]:
        if not line.startswith('|'):
            break
        cols = split_row(line)
        if not cols:
            continue
        rows.append(cols)

    return lines, header_idx, header, sep, rows


def write_report(lines, header_idx, sep, rows):
    out = []
    out.extend(lines[:header_idx])
    out.append(lines[header_idx])
    out.append(sep)
    for cols in rows:
        out.append('| ' + ' | '.join(cols) + ' |')
    out.extend(lines[header_idx + 2 + len(rows):])
    REPORT_PATH.write_text('\n'.join(out) + '\n', encoding='utf-8')


def run_compare(url):
    env = os.environ.copy()
    env['TAO_MP3_COMPARE_INPUT'] = url
    cmd = [
        'cargo',
        'test',
        '--test',
        'mp3_module_compare',
        '--',
        '--nocapture',
        '--ignored',
    ]
    proc = subprocess.run(cmd, text=True, capture_output=True, env=env)
    output = (proc.stdout or '') + '\n' + (proc.stderr or '')
    return proc.returncode, output


def parse_metrics(output):
    for line in output.splitlines():
        if 'Tao对比样本=' in line and 'Tao/FFmpeg:' in line:
            m = LINE_RE.search(line)
            if not m:
                continue
            tao_samples = int(m.group(2))
            ff_samples = int(m.group(3))
            return {
                'tao_samples': tao_samples,
                'ff_samples': ff_samples,
                'sample_diff': tao_samples - ff_samples,
                'max_err': m.group(4),
                'psnr': m.group(5),
                'precision': m.group(6),
            }
    return None


def extract_failure_reason(output):
    lines = [ln.strip() for ln in output.splitlines() if ln.strip()]
    if not lines:
        return '无输出'
    tail = lines[-3:]
    return ' | '.join(tail)


def main():
    urls = read_urls()
    lines, header_idx, header, sep, rows = load_report()

    col_map = {name: idx for idx, name in enumerate(header)}
    required = ['序号', 'URL', '状态', '失败原因', 'Tao样本数', 'FFmpeg样本数', '样本数差异', 'max_err', 'psnr(dB)', '精度(%)', '备注']
    for name in required:
        if name not in col_map:
            raise RuntimeError(f'报告表缺少列: {name}')

    total = len(urls)
    for idx, url in enumerate(urls, 1):
        if idx - 1 >= len(rows):
            raise RuntimeError('报告行数不足, 请重新生成报告模板.')
        row = rows[idx - 1]
        status = row[col_map['状态']]
        if status in ('成功', '失败'):
            continue

        print(f'开始处理 {idx}/{total}: {url}')
        code, output = run_compare(url)

        if code == 0:
            metrics = parse_metrics(output)
            if metrics is None:
                row[col_map['状态']] = '失败'
                row[col_map['失败原因']] = '未找到对比输出行'
            else:
                row[col_map['状态']] = '成功'
                row[col_map['失败原因']] = ''
                row[col_map['Tao样本数']] = str(metrics['tao_samples'])
                row[col_map['FFmpeg样本数']] = str(metrics['ff_samples'])
                row[col_map['样本数差异']] = str(metrics['sample_diff'])
                row[col_map['max_err']] = metrics['max_err']
                row[col_map['psnr(dB)']] = metrics['psnr']
                row[col_map['精度(%)']] = metrics['precision']
        else:
            row[col_map['状态']] = '失败'
            row[col_map['失败原因']] = extract_failure_reason(output)

        rows[idx - 1] = row
        write_report(lines, header_idx, sep, rows)
        print(f'已记录 {idx}/{total}: {row[col_map["状态"]]}')

    print('处理完成.')


if __name__ == '__main__':
    main()
