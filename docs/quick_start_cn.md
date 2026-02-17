# 快速开始指南 (Quick Start Guide)

## 前置要求

*   Rust (最新稳定版)
*   Cargo

## 安装

在你的 `Cargo.toml` 中添加 `tao`：

```toml
[dependencies]
tao = "0.1.0" # 请检查 crates.io 获取最新版本
```

## 基本用法

### Tao Probe (类似 ffprobe)

```bash
cargo run -p tao-probe -- input.mp4
```

### Tao Play (类似 ffplay)

```bash
cargo run -p tao-play -- input.mp4
```

### 库使用

```rust
use tao::prelude::*;

fn main() {
    // 更多用法请参考 examples/ 目录
}
```
