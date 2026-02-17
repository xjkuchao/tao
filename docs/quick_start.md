# Quick Start Guide

## Prerequisites

*   Rust (latest stable version)
*   Cargo

## Installation

Add `tao` to your `Cargo.toml`:

```toml
[dependencies]
tao = "0.1.0" # Check crates.io for the latest version
```

## Basic Usage

### Tao Probe (Similar to ffprobe)

```bash
cargo run -p tao-probe -- input.mp4
```

### Tao Play (Similar to ffplay)

```bash
cargo run -p tao-play -- input.mp4
```

### Library Usage

```rust
use tao::prelude::*;

fn main() {
    // See examples/ directory for more usage
}
```

[Click here for the Chinese version (中文版)](quick_start_cn.md)
