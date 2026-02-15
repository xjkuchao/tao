# tao-play egui 迁移说明

## 概述

成功将 `tao-play` 从 minifb 迁移到 egui/eframe，以解决 Wayland 下的 CSR (Client-Side Decoration) 问题。

## 主要改动

### 1. 依赖更新 (`bins/tao-play/Cargo.toml`)

- **移除**: `minifb = "0.28"`
- **添加**:
    - `eframe = "0.26"`
    - `egui = "0.26"`

### 2. 架构重构

#### 新增文件

- **`src/gui.rs`**: egui 应用界面
    - `PlayerApp` 结构体实现 `eframe::App` trait
    - 处理视频渲染、进度条、OSD 显示
    - 键盘控制 (空格暂停、ESC/Q 退出、方向键控制等)

#### 修改文件

- **`src/player.rs`**: 播放器核心逻辑重构
    - 从同步运行改为异步后台线程运行
    - 新增消息通道通信:
        - `frame_tx/frame_rx`: 发送视频帧到 GUI
        - `status_tx/status_rx`: 发送播放状态 (时间、音量、暂停等)
        - `command_tx/command_rx`: 接收 GUI 控制命令
    - 新增 `VideoFrame` 结构体 (RGB24 数据)
    - 新增 `PlayerCommand` 和 `PlayerStatus` 枚举

- **`src/main.rs`**: 入口重构
    - 使用 `eframe::run_native()` 启动 GUI
    - 创建播放器后台线程
    - 建立通道连接 GUI 和播放器

#### 删除文件

- **`src/video.rs`**: 旧的 minifb 视频显示模块 (已废弃)

### 3. 功能实现

#### GUI 功能

- ✅ 视频帧显示 (自动适配窗口大小，保持宽高比)
- ✅ 进度条显示
- ✅ OSD 文本提示
- ✅ 深色主题
- ✅ 键盘控制:
    - 空格: 暂停/继续
    - 左/右箭头: 快退/快进 (±10秒)
    - 上/下箭头: 音量 +/-
    - M: 静音切换
    - ESC/Q: 退出

#### Wayland 支持

- ✅ egui/eframe 原生支持 Wayland
- ✅ 支持 CSR (Client-Side Decoration)
- ✅ 可以调整窗口大小、移动窗口
- ✅ 正常显示标题栏

## 构建和运行

```bash
# 构建
cargo build -p tao-play --release

# 运行
./target/release/tao-play <video_file>

# 示例
./target/release/tao-play test_videos/sample.avi
```

## 已知问题/TODO

1. **Seek 功能未完全实现**: 目前只是发送命令，demuxer 层面的 seek 还需要实现
2. **性能优化**: 每帧都重新上传纹理，可以优化为仅在新帧到达时上传
3. **错误处理**: `PlayerStatus::Error` 未实际使用

## 技术细节

- **线程模型**:
    - 主线程运行 egui 事件循环
    - 后台线程运行播放器 (demux + decode + A/V sync)
    - 通过 `mpsc::channel` 通信

- **视频同步**: 继续使用音频时钟为基准，视频帧按 PTS 延迟显示

- **纹理管理**: 使用 `egui::TextureHandle`，每帧调用 `ctx.load_texture()` 更新

## 测试建议

在 Wayland 环境下测试:

```bash
export WAYLAND_DISPLAY=wayland-0
./target/release/tao-play <video_file>
```

检查:

- [ ] 窗口是否有标题栏
- [ ] 能否拖动窗口
- [ ] 能否调整窗口大小
- [ ] 视频播放流畅性
- [ ] 键盘控制是否正常
