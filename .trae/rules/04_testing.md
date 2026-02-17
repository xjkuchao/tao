# 测试与性能

## 测试规范

- 修改后必须执行 `cargo check` 与 `cargo test`；新增编解码器或容器格式必须编写测试  
- 集成测试在 `tests/`；单元测试在源文件 `#[cfg(test)]` 模块  
- 所有样本使用 HTTPS URL 直接访问，不下载到本地；临时文件在 `data/` 并被 Git 忽略  
- 覆盖正常/边界/错误场景；示例用例与命名规则；帧数限制 5-10 帧加快测试

---

## 手动播放测试规范（始终生效）

- 播放时长限制：默认 10 秒，必要时最多 30 秒；结束后主动终止  
- 强制超时保护：使用 `timeout` 包裹 `tao-play`（默认 30 秒；短测 15 秒）  
- Linux/macOS 使用 `timeout`；Windows 使用 `Start-Process` + 定时 `TASKKILL`  
- 超时后强制终止为预期行为；检查日志分析  
- 终止进程规范（Windows 使用 `TASKKILL /F /IM tao-play.exe /T`；Linux/macOS 使用 `pkill/killall`）  
- 流式播放测试统一使用 URL

---

## 测试文件与临时文件管理

- 核心原则：样本用 URL；临时文件放 `data/` 且被 Git 忽略；遵循 `samples/SAMPLES.md`
- 目录结构与文件位置约定（tests/benches/samples/data/logs 等）
- 测试命名规范；样本来源/清单与用法；临时文件的创建/命名/清理（RAII 示例）
- 日志保存位置与前缀规范；AI 调试指南
- 清理 `data/` 的跨平台示例；网络/清理/磁盘空间/安全注意事项

---

## 性能优化指南

- 内存：避免不必要分配；缓冲区复用；零拷贝传递（`bytes::Bytes`）  
- 数据处理：迭代器优先；热路径避免分支失败；考虑 SIMD  
- 性能测试：`benches/` + `cargo bench`；使用 profiler 分析瓶颈
