#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

MATRIX_FILE="${1:-bins/tao-probe/tests/compat_command_matrix.txt}"
TP_BIN="${TP_BIN:-target/debug/tao-probe}"
DIFF_LINES="${DIFF_LINES:-60}"

if [[ ! -f "$MATRIX_FILE" ]]; then
    echo "[matrix] 命令矩阵文件不存在: $MATRIX_FILE" >&2
    exit 1
fi

if ! command -v ffprobe >/dev/null 2>&1; then
    echo "[matrix] 未找到 ffprobe, 无法执行对拍." >&2
    exit 1
fi

if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "[matrix] 未找到 ffmpeg, 无法构造样本." >&2
    exit 1
fi

if [[ ! -x "$TP_BIN" ]]; then
    cargo build -p tao-probe >/dev/null
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
SAMPLE_FILE="$TMP_DIR/sample.wav"
ffmpeg -f lavfi -i anullsrc=r=8000:cl=mono -t 0.01 -acodec pcm_s16le -y -loglevel quiet "$SAMPLE_FILE"

PASS=0
FAIL=0
INDEX=0

normalize_output() {
    local input_file="$1"
    local output_file="$2"
    sed -E \
        -e 's/0x[0-9a-fA-F]+/0xADDR/g' \
        "$input_file" >"$output_file"
}

while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
    line="$(echo "$raw_line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    if [[ -z "$line" || "${line:0:1}" == "#" ]]; then
        continue
    fi

    cmd="${line//\{SAMPLE\}/$SAMPLE_FILE}"
    INDEX=$((INDEX + 1))
    tao_out="$TMP_DIR/tao.out"
    tao_err="$TMP_DIR/tao.err"
    ff_out="$TMP_DIR/ff.out"
    ff_err="$TMP_DIR/ff.err"
    tao_out_norm="$TMP_DIR/tao.out.norm"
    tao_err_norm="$TMP_DIR/tao.err.norm"
    ff_out_norm="$TMP_DIR/ff.out.norm"
    ff_err_norm="$TMP_DIR/ff.err.norm"

    set +e
    eval "\"$TP_BIN\" $cmd" >"$tao_out" 2>"$tao_err"
    tao_code=$?
    eval "ffprobe $cmd" >"$ff_out" 2>"$ff_err"
    ff_code=$?
    set -e

    normalize_output "$tao_out" "$tao_out_norm"
    normalize_output "$tao_err" "$tao_err_norm"
    normalize_output "$ff_out" "$ff_out_norm"
    normalize_output "$ff_err" "$ff_err_norm"

    if [[ $tao_code -eq $ff_code ]] \
        && cmp -s "$tao_out_norm" "$ff_out_norm" \
        && cmp -s "$tao_err_norm" "$ff_err_norm"; then
        PASS=$((PASS + 1))
        printf "[PASS][%03d] %s\n" "$INDEX" "$cmd"
    else
        FAIL=$((FAIL + 1))
        printf "[FAIL][%03d] %s\n" "$INDEX" "$cmd"
        printf "  code tao/ffprobe: %s/%s\n" "$tao_code" "$ff_code"
        if ! cmp -s "$tao_out_norm" "$ff_out_norm"; then
            echo "  stdout diff:"
            diff -u "$ff_out_norm" "$tao_out_norm" | sed -n "1,${DIFF_LINES}p" || true
        fi
        if ! cmp -s "$tao_err_norm" "$ff_err_norm"; then
            echo "  stderr diff:"
            diff -u "$ff_err_norm" "$tao_err_norm" | sed -n "1,${DIFF_LINES}p" || true
        fi
    fi
done <"$MATRIX_FILE"

echo "[matrix] pass=$PASS fail=$FAIL"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
