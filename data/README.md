# Tao 数据目录

本目录包含 Tao 项目的测试数据文件和临时文件。

## 目录结构

```
data/
├── SAMPLE_URLS.md    # 测试样本 URL 清单
├── test/             # 测试数据文件
│   ├── unit/         # 单元测试数据
│   ├── integration/  # 集成测试数据
│   └── bench/        # 基准测试数据
└── tmp/              # 临时文件目录 (不提交到 Git)
```

## 文件管理规则

### 样本文件 (SAMPLE_URLS.md)

- **所有样本使用 URL 方式访问，不下载到本地**
- 样本来源: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
- 样本清单: `SAMPLE_URLS.md` 记录所有测试样本的 URL 和用途
- 测试用例直接使用 URL 创建 Demuxer/Decoder，无需下载文件

### 测试数据 (test/)

- 单元测试和集成测试所需的数据文件
- 所有测试数据提交到 Git 版本控制
- 按测试类型分类存放

### 临时文件 (tmp/)

- 运行时生成的临时文件
- 编解码过程中的中间文件
- **永不提交到 Git**

## 使用示例

### 从 URL 清单查找样本

```rust
// 1. 打开 data/SAMPLE_URLS.md
// 2. 搜索对应编解码器 (如 "H.264")
// 3. 找到合适的 URL

#[test]
fn test_h264_decode() {
    // 直接使用 URL (从 SAMPLE_URLS.md 复制)
    let sample_url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    let mut demuxer = DemuxerRegistry::open(sample_url).unwrap();
    // ... 测试代码
}
```

### 创建临时文件

```rust
use std::path::PathBuf;

let temp_dir = PathBuf::from("data/tmp");
std::fs::create_dir_all(&temp_dir).unwrap();
let temp_file = temp_dir.join(format!("tmp_test_{}.bin", std::process::id()));
```

## 添加新样本

### 查找样本

1. 访问 https://samples.ffmpeg.org/ 浏览样本库
2. 或查看样本列表:
    - https://samples.ffmpeg.org/allsamples.txt
    - https://samples.ffmpeg.org/allsamples-old.txt

### 验证样本

使用 `ffprobe` 验证样本信息:

```bash
ffprobe https://samples.ffmpeg.org/path/to/sample.mp4
```

### 添加到清单

在 `SAMPLE_URLS.md` 对应章节添加:

```markdown
| 用途描述 | https://samples.ffmpeg.org/path/to/sample.ext | 详细说明 |
```

### 提交更新

```bash
git add data/SAMPLE_URLS.md
git commit -m "docs: 添加 XXX 编解码器测试样本 URL"
```

## 环境变量

可以通过 `TAO_DATA_DIR` 环境变量指定数据目录的绝对路径:

```bash
export TAO_DATA_DIR="/path/to/tao/data"
```

## 清理临时文件

临时文件会在测试结束后自动清理，如需手动清理:

```bash
rm -rf data/tmp/*
```

## 注意事项

- 确保所有测试代码中使用 URL 而非本地路径
- 临时文件使用进程 ID 或时间戳命名避免冲突
- 所有样本通过 URL 访问, 确保测试环境有网络连接
- 测试只解码前几帧 (5-10 帧) 验证功能即可
- URL 失效时从 https://samples.ffmpeg.org/ 查找替代样本
