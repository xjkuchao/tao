#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

collect_changed_files() {
    {
        git diff --name-only
        git diff --name-only --cached
        git ls-files --others --exclude-standard
    } | sed '/^$/d' | sort -u
}

mapfile -t CHANGED_FILES < <(collect_changed_files)
STRICT_MODE="${STRICT_MODE:-0}"

if [[ ${#CHANGED_FILES[@]} -eq 0 ]]; then
    echo "[scope-check] 工作区无变更."
    exit 0
fi

declare -a OUT_OF_SCOPE=()
for file in "${CHANGED_FILES[@]}"; do
    if [[ "$file" != bins/tao-probe/* ]]; then
        OUT_OF_SCOPE+=("$file")
    fi
done

if [[ ${#OUT_OF_SCOPE[@]} -gt 0 ]]; then
    echo "[scope-check] 检测到工作区存在超出范围的变更:"
    for file in "${OUT_OF_SCOPE[@]}"; do
        echo "  - $file"
    done
    if [[ "$STRICT_MODE" == "1" ]]; then
        echo "[scope-check] 严格模式: 仅允许修改 bins/tao-probe/**."
        exit 1
    fi
    echo "[scope-check] 已忽略超出范围变更(默认模式), 仅关注 bins/tao-probe/**."
    exit 0
fi

echo "[scope-check] 通过: 变更全部位于 bins/tao-probe/**."
