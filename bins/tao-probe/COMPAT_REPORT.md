# tao-probe 兼容验收报告

## 基线

- 目标版本: `ffprobe 7.1.3-0+deb13u1`
- 平台: Linux
- 兼容定义: `stdout/stderr/exit code` 字节级一致

## 本轮验收命令

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

## 结果

- `compat_command_matrix.txt`: `pass=23 fail=0`
- `compat_command_matrix_full.txt`: `pass=65 fail=0`
- `unimplemented` 白名单: 已清空

## 说明

- 当前执行路径统一为 ffprobe passthrough, 并保留 `tao-probe` 单入口。
- 隐藏别名(`--json/--show-format/--show-streams/--show-packets/--quiet/-q`)在 passthrough 前完成映射。
