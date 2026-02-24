#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

ROUND_ID="${1:-}"
ROUND_NOTE="${2:-}"
if [[ -z "$ROUND_ID" ]]; then
    echo "用法: $0 <轮次ID> [轮次说明]"
    echo "示例: $0 R1 \"B_Direct temporal 映射一致性\""
    exit 1
fi

SAMPLE_PATH="${TAO_ROUND_SAMPLE:-data/1_h264.mp4}"
TEST_TARGET="${TAO_ROUND_TEST_TARGET:-h264::test_h264_compare_sample_1}"

LOG_DIR="data/h264_round_logs"
mkdir -p "$LOG_DIR"

BEST_FILE="data/h264_round_logs/best_score.env"
JOURNAL_FILE="plans/tao-codec/video/h264/round_journal.md"
SKIP_STRICT="${TAO_ROUND_SKIP_STRICT:-0}"

if [[ ! -f "$BEST_FILE" ]]; then
    cat >"$BEST_FILE" <<'EOF'
BEST_P299=0
BEST_FM=-1
BEST_P67=0
BEST_P10=0
BEST_ROUND=INIT
EOF
fi

source "$BEST_FILE"

run_compare() {
    local frames="$1"
    local extra_env="$2"
    local log_file="$LOG_DIR/${ROUND_ID}_f${frames}.log"
    echo "[$ROUND_ID] 运行 FRAMES=$frames ..."
    eval "TAO_H264_COMPARE_INPUT=$SAMPLE_PATH TAO_H264_COMPARE_FRAMES=$frames TAO_H264_COMPARE_REQUIRED_PRECISION=0 $extra_env cargo test --release --test run_decoder $TEST_TARGET -- --nocapture --ignored" \
        | tee "$log_file"
}

extract_score_from_log() {
    local log_file="$1"
    local expected_frames="$2"
    local score_line
    score_line="$(rg "SCORE precision=.*frames=${expected_frames}" "$log_file" | tail -n 1 || true)"
    if [[ -z "$score_line" ]]; then
        echo "0 -1"
        return
    fi
    local precision mismatch
    precision="$(sed -E 's/.*precision=([0-9.]+).*/\1/' <<<"$score_line")"
    mismatch="$(sed -E 's/.*first_mismatch=([-0-9]+).*/\1/' <<<"$score_line")"
    echo "$precision $mismatch"
}

is_better_score() {
    local p299="$1"
    local fm="$2"
    local p67="$3"
    local p10="$4"
    awk -v p299="$p299" -v bp299="$BEST_P299" \
        -v fm="$fm" -v bfm="$BEST_FM" \
        -v p67="$p67" -v bp67="$BEST_P67" \
        -v p10="$p10" -v bp10="$BEST_P10" '
        function fm_rank(v) { return (v < 0 ? 1000000000 : v) }
        BEGIN {
            eps = 1e-9
            if (p299 > bp299 + eps) { print 1; exit }
            if (bp299 > p299 + eps) { print 0; exit }
            if (fm_rank(fm) > fm_rank(bfm)) { print 1; exit }
            if (fm_rank(bfm) > fm_rank(fm)) { print 0; exit }
            if (p67 > bp67 + eps) { print 1; exit }
            if (bp67 > p67 + eps) { print 0; exit }
            if (p10 > bp10 + eps) { print 1; exit }
            print 0
        }'
}

run_strict_validation() {
    local validate_log="$LOG_DIR/${ROUND_ID}_strict_validation.log"
    : >"$validate_log"
    local cmds=(
        "cargo fmt --all -- --check"
        "cargo clippy --workspace --all-targets --all-features -- -D warnings"
        "cargo check --workspace --all-targets --all-features"
        "cargo test --workspace --all-targets --all-features --no-fail-fast"
        "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps"
    )
    local failed=0
    for cmd in "${cmds[@]}"; do
        echo "[${ROUND_ID}] 严格验证: $cmd" | tee -a "$validate_log"
        if ! bash -lc "$cmd" 2>&1 | tee -a "$validate_log"; then
            failed=1
        fi
    done
    return "$failed"
}

run_compare 3 "TAO_H264_COMPARE_ANALYZE_FRAME_STATS=1 TAO_H264_COMPARE_SKIP_DEBLOCK=1 TAO_SKIP_DEBLOCK=1"
run_compare 10 ""
run_compare 67 ""
run_compare 299 "TAO_H264_COMPARE_TIMING=1"

read -r P3 FM3 < <(extract_score_from_log "$LOG_DIR/${ROUND_ID}_f3.log" 3)
read -r P10 FM10 < <(extract_score_from_log "$LOG_DIR/${ROUND_ID}_f10.log" 10)
read -r P67 FM67 < <(extract_score_from_log "$LOG_DIR/${ROUND_ID}_f67.log" 67)
read -r P299 FM299 < <(extract_score_from_log "$LOG_DIR/${ROUND_ID}_f299.log" 299)

BETTER="$(is_better_score "$P299" "$FM299" "$P67" "$P10")"

echo
echo "[$ROUND_ID] 分数汇总:"
echo "  G0(F3):   precision=$P3 first_mismatch=$FM3"
echo "  G1(F10):  precision=$P10 first_mismatch=$FM10"
echo "  G2(F67):  precision=$P67 first_mismatch=$FM67"
echo "  G3(F299): precision=$P299 first_mismatch=$FM299"
echo "  当前最优: P299=$BEST_P299 FM=$BEST_FM P67=$BEST_P67 P10=$BEST_P10 (round=$BEST_ROUND)"

if [[ "$BETTER" == "1" ]]; then
    STRICT_RESULT="未执行"
    if [[ "$SKIP_STRICT" == "1" ]]; then
        echo "[$ROUND_ID] 判定: 本轮分数提升, 当前配置跳过严格验证(TAO_ROUND_SKIP_STRICT=1)."
        STRICT_RESULT="跳过"
    else
        echo "[$ROUND_ID] 判定: 本轮分数提升, 触发 5 项严格验证."
        if run_strict_validation; then
            STRICT_RESULT="通过"
        else
            STRICT_RESULT="失败"
        fi
    fi
    cat >"$BEST_FILE" <<EOF
BEST_P299=$P299
BEST_FM=$FM299
BEST_P67=$P67
BEST_P10=$P10
BEST_ROUND=$ROUND_ID
EOF
    RESULT_TEXT="提升(${STRICT_RESULT})"
else
    echo "[$ROUND_ID] 判定: 本轮未形成明确提升, 应回滚实验改动并进入下一轮."
    RESULT_TEXT="未提升"
fi

if [[ ! -f "$JOURNAL_FILE" ]]; then
    cat >"$JOURNAL_FILE" <<'EOF'
# H264 精度轮转执行日志

| 时间(UTC) | 轮次 | 样本 | 说明 | G3 P299 | 首个不一致帧 | G2 P67 | G1 P10 | 结论 |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- |
EOF
fi

NOW_UTC="$(date -u '+%Y-%m-%d %H:%M:%S')"
printf '| %s | %s | `%s` | %s | %.6f | %s | %.6f | %.6f | %s |\n' \
    "$NOW_UTC" "$ROUND_ID" "$SAMPLE_PATH" "${ROUND_NOTE:--}" \
    "$P299" "$FM299" "$P67" "$P10" "$RESULT_TEXT" >>"$JOURNAL_FILE"

echo "[$ROUND_ID] 日志已写入:"
echo "  - $JOURNAL_FILE"
echo "  - $LOG_DIR/${ROUND_ID}_f3.log"
echo "  - $LOG_DIR/${ROUND_ID}_f10.log"
echo "  - $LOG_DIR/${ROUND_ID}_f67.log"
echo "  - $LOG_DIR/${ROUND_ID}_f299.log"
