# tao-probe 兼容对拍本地流程

本文件仅描述 `bins/tao-probe/**` 的本地兼容门禁流程。

## 快速命令

```bash
STRICT_MODE=1 bins/tao-probe/scripts/check_scope.sh
cargo fmt -p tao-probe -- --check
cargo clippy -p tao-probe --all-targets --all-features --no-deps -- -D warnings
cargo check -p tao-probe --all-targets --all-features
cargo test -p tao-probe --all-targets --all-features --no-fail-fast
RUSTDOCFLAGS="-D warnings" cargo doc -p tao-probe --all-features --no-deps
bins/tao-probe/scripts/compat_matrix.sh bins/tao-probe/tests/compat_command_matrix.txt
bins/tao-probe/scripts/compat_matrix.sh bins/tao-probe/tests/compat_command_matrix_full.txt
```

## 说明

1. 仅检查 `tao-probe` crate, 不处理 workspace 其他目录告警。
2. `compat_matrix.sh` 默认会规范化动态地址字段(`0x...`), 并在失败时输出差异摘要。
3. `compat_command_matrix_full.txt` 用于顺序敏感和参数边界回归。
