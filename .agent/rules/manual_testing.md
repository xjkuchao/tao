# 手动播放测试规范

## 16.1 播放时长限制

- 手动测试音视频播放时, **禁止完整播放**整个文件.
- 默认播放 **前 10 秒** 即可验证功能, 如有必要 (如需验证 seek/后段内容) 可增加到 **最多 30 秒**.
- 播放结束后必须主动终止播放进程.

## 16.2 终止播放进程 (Windows)

- Windows 下终止 tao-play 进程时, **必须使用 `TASKKILL /F /IM tao-play.exe /T`**.
- **禁止使用 `TASKKILL /F /PID <pid>`**, 因为在 Cursor/shell 环境中通常无法获取到正确的 PID.
- 示例:
    ```powershell
    # 正确
    TASKKILL /F /IM tao-play.exe /T
    # 错误 (PID 不可靠)
    TASKKILL /F /PID 12345
    ```

## 16.3 流式播放测试

- `tao-play` 支持 http/https/rtmp 等流式 URL 播放.
- 测试在线音视频文件时, **必须使用 URL 直接流式播放**, 不要先下载到本地再播放.
- 仅当需要反复使用同一文件 (如单元测试/集成测试数据) 时, 才下载到 `data/samples/` 目录.
- 示例:
    ```powershell
    # 正确: 直接流式播放
    cargo run --package tao-play -- "https://samples.ffmpeg.org/flac/Yesterday.flac"
    # 错误: 先下载再播放
    curl -o data/samples/audio/test.flac "https://samples.ffmpeg.org/flac/Yesterday.flac"
    cargo run --package tao-play -- "data/samples/audio/test.flac"
    ```
