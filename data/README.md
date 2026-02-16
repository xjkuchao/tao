# Tao 数据目录

本目录包含 Tao 项目的测试文件、样本文件和临时文件。

## 目录结构

```
data/
├── samples/          # 测试样本文件
│   ├── video/        # 视频样本文件
│   ├── audio/        # 音频样本文件
│   └── container/    # 容器格式样本文件
├── test/             # 测试数据文件
│   ├── unit/         # 单元测试数据
│   ├── integration/  # 集成测试数据
│   └── bench/        # 基准测试数据
└── tmp/              # 临时文件目录 (不提交到 Git)
```

## 文件管理规则

### 样本文件 (samples/)
- 用于测试各种编解码器和容器格式
- 所有样本文件提交到 Git (包括大文件)
- 样本来源: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
- 文件命名使用描述性名称，如 `h264_test.mp4`, `theora_sample.ogg`
- 样本清单和下载计划参见 `SAMPLES.md`

### 测试数据 (test/)
- 单元测试和集成测试所需的数据文件
- 所有测试数据提交到 Git 版本控制
- 按测试类型分类存放

### 临时文件 (tmp/)
- 运行时生成的临时文件
- 下载的测试文件
- 编解码过程中的中间文件
- **永不提交到 Git**

## 使用示例

```rust
// 在测试中使用样本文件
#[test]
fn test_theora_decoder() {
    let sample_path = "data/samples/video/theora_test.ogg";
    // 测试代码...
}

// 创建临时文件
use std::path::PathBuf;
let temp_dir = PathBuf::from("data/tmp");
std::fs::create_dir_all(&temp_dir).unwrap();
let temp_file = temp_dir.join(format!("tmp_test_{}.bin", std::process::id()));
```

## 样本文件管理

- **下载脚本**: 使用 `download_samples.ps1` 自动下载测试样本
- **样本清单**: 参见 `SAMPLES.md` 了解样本列表和下载计划
- **样本来源**: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)

## 环境变量

可以通过 `TAO_DATA_DIR` 环境变量指定数据目录的绝对路径：

```bash
export TAO_DATA_DIR="/path/to/tao/data"
```

## 清理临时文件

临时文件会在测试结束后自动清理，如需手动清理：

```bash
rm -rf data/tmp/*
```

## 注意事项

- 确保所有测试代码中使用相对路径
- 临时文件使用进程 ID 或时间戳命名避免冲突
- 所有样本文件提交到 Git 以确保测试可复现性
- 定期清理临时文件目录
