# 测试文件和临时文件管理

## 17.1 目录结构

- **`data/`**: 所有测试用文件和数据的根目录
    - **`data/samples/`**: 测试样本文件 (如测试视频、音频文件)
    - **`data/test/`**: 单元测试和集成测试所需的数据文件
    - **`data/tmp/`**: 临时文件目录

## 17.2 文件放置规则

- **测试样本文件**: 必须放在 `data/samples/` 目录下
    - 按格式分类: `data/samples/video/`, `data/samples/audio/`, `data/samples/container/`
    - 文件命名使用描述性名称, 如 `h264_test.mp4`, `theora_sample.ogg`
- **测试数据文件**: 必须放在 `data/test/` 目录下
    - 单元测试数据: `data/test/unit/`
    - 集成测试数据: `data/test/integration/`
    - 基准测试数据: `data/test/bench/`
- **临时文件**: 必须放在 `data/tmp/` 目录下
    - 运行时生成的临时文件
    - 下载的测试文件
    - 编解码过程中的中间文件

## 17.3 临时文件管理

- **创建**: 所有临时文件必须在 `data/tmp/` 目录下创建
- **清理**: 测试结束后必须清理临时文件
- **命名**: 临时文件使用前缀 `tmp_` 或进程 ID 命名
- **权限**: 确保临时文件有适当的读写权限

## 17.4 Git 管理

- **`data/samples/`**: 小文件 (< 1MB) 可提交到 Git, 大文件使用 Git LFS
- **`data/test/`**: 测试数据文件可提交到 Git
- **`data/tmp/`**: 永不提交到 Git, 必须在 `.gitignore` 中排除

## 17.5 代码规范

- **路径使用**: 在代码中使用相对于项目根目录的路径
- **环境变量**: 可使用环境变量 `TAO_DATA_DIR` 指定数据目录
- **错误处理**: 文件不存在时提供清晰的错误信息
- **跨平台**: 确保路径处理在 Windows/Linux/macOS 上兼容
