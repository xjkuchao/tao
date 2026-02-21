# H264 解码器 -- 性能优化计划

> 前置条件: `decoder_accuracy.md` 精度目标**全部达标**后才允许进入本计划.
>
> 关联文档:
> - 功能开发: `decoder_dev.md`
> - 精度收敛: `decoder_accuracy.md`

## 1. 目标

- SIMD 优化后, 单线程解码吞吐较纯标量提升 >= 3 倍.
- 多线程优化后, 4 核解码吞吐较单线程提升 >= 2 倍.
- 内存优化后, 峰值内存降低 >= 20%.
- 全部优化必须与标量参考路径 **bit-exact**, 不得引入精度回归.

## 2. Benchmark 基础设施

- [ ] 新建 `benches/h264_decode_bench.rs`, 使用 `criterion` 建立解码吞吐基线.
- [ ] 按模块拆分 micro-benchmark: IDCT / 运动补偿 / 帧内预测 / 去块滤波.
- [ ] 记录基线到 `plans/tao-codec/video/h264/perf_baseline.md`.
- [ ] 测试样本: 1080p High Profile 标准码流, 解码 100+ 帧取平均.

## 3. P8 SIMD 优化

### P8.1 抽象层

- [ ] 运行时 CPU 特性检测:
    - x86: `std::arch::is_x86_feature_detected!("sse2"/"avx2")`.
    - AArch64: `std::arch::is_aarch64_feature_detected!("neon")`.
- [ ] 函数指针表(DSP context): 启动时按 CPU 能力选择最优路径.
- [ ] Trait 接口: `H264Dsp` / `H264Qpel` / `H264Pred` / `H264Deblock`.
- [ ] `checkasm` 风格测试: SIMD vs 标量结果自动一致性比对.

### P8.2 IDCT 反变换

- [ ] 4x4 IDCT SSE2.
- [ ] 8x8 IDCT SSE2/AVX2.
- [ ] 4x4 Hadamard SSE2.
- [ ] AArch64 NEON: 4x4/8x8 IDCT + Hadamard.
- [ ] SIMD vs 标量一致性单测.

### P8.3 运动补偿

- [ ] 亮度 6-tap 半像素(水平 + 垂直) SSE2/AVX2.
- [ ] 亮度 1/4 像素平均 SSE2/AVX2.
- [ ] 色度 1/8 双线性 SSE2/AVX2.
- [ ] 加权预测(单向/双向) SSE2/AVX2.
- [ ] AArch64 NEON: 亮度 6-tap / 色度双线性 / 加权.
- [ ] SIMD vs 标量一致性单测.

### P8.4 帧内预测

- [ ] Intra 16x16 DC/V/H/Plane SSE2.
- [ ] Intra 4x4 DC SSE2(批量).
- [ ] 色度 DC/V/H/Plane SSE2.
- [ ] AArch64 NEON: 帧内预测.
- [ ] SIMD vs 标量一致性单测.

### P8.5 去块滤波

- [ ] 亮度垂直边界 SSE2/AVX2(16 行一组).
- [ ] 亮度水平边界 SSE2/AVX2.
- [ ] 色度边界 SSE2.
- [ ] 强滤波(bs=4) SIMD.
- [ ] AArch64 NEON: 去块滤波.
- [ ] SIMD vs 标量一致性单测.

- **P8 验收**: 全部 SIMD 与标量 bit-exact, benchmark 吞吐 >= 3x.

## 4. P9 多线程解码

### P9.1 Slice 级并行

- [ ] 同帧多 slice 独立线程并行.
- [ ] 宏块行级同步(行进度标记).
- [ ] 各 slice 写入共享帧缓冲(按宏块地址分区).
- [ ] 正确性单测(结果与单线程一致).

### P9.2 帧级并行

- [ ] 参考依赖跟踪(每帧记录"已解码行数"进度).
- [ ] 线程池管理(`rayon` 或自定义).
- [ ] DPB 线程安全(`Arc<RwLock<>>` 或无锁).
- [ ] 正确性单测(结果与单线程一致).

### P9.3 熵解码与重建分离

- [ ] 熵解码阶段: 纯语法解析, 输出宏块描述符.
- [ ] 重建阶段: 预测+残差+去块, 可并行化.
- [ ] 适用于高分辨率码流的熵瓶颈场景.

- **P9 验收**: 多线程与单线程 bit-exact, 4 核吞吐 >= 2x.

## 5. P10 内存与缓存优化

### P10.1 内存分配

- [ ] DPB 帧缓冲池: 预分配 `max_dpb_frames + 1`, 解码完成归还.
- [ ] 宏块临时缓冲: 栈分配或 Vec 复用, 避免热路径 heap 分配.
- [ ] 参考帧零拷贝: `Arc<Vec<u8>>` 或 `bytes::Bytes` 共享.
- [ ] 内存热点 profiling + 优化前后对比.

### P10.2 缓存友好布局

- [ ] 宏块状态紧凑化(AoS/SoA 混合, 减少 cache line 跨越).
- [ ] 帧缓冲 64 字节对齐 + 行步长 cache line 对齐.
- [ ] 运动补偿预取(prefetch).

### P10.3 位流读取

- [ ] CABAC `decode_decision()` 内联 + 分支预测提示.
- [ ] CABAC `renormalize()` 使用 `leading_zeros` 批量归一化.
- [ ] CAVLC VLC 表紧凑数组 + 多 bit 批量读取.

- **P10 验收**: 单线程吞吐在 P8 基础上再提升 10-30%, 内存峰值降低 20%+.

## 6. P11 最终门禁

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo check --workspace --all-targets --all-features`
- [ ] `cargo test --workspace --all-targets --all-features --no-fail-fast`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- [ ] 精度回归 CI 通过(全样本 PSNR >= 50dB).
- [ ] 性能数据汇总表(标量 vs SIMD vs 多线程).
- [ ] 输出最终报告(偏差总结 + 性能数据 + 风险 + 剩余事项).
- [ ] 更新 `h264_feature_matrix.md` 最终状态.

## 7. 进度

- [ ] Benchmark 基础设施
- [ ] P8 SIMD 优化
    - [ ] P8.1 抽象层
    - [ ] P8.2 IDCT
    - [ ] P8.3 运动补偿
    - [ ] P8.4 帧内预测
    - [ ] P8.5 去块滤波
- [ ] P9 多线程
    - [ ] P9.1 Slice 并行
    - [ ] P9.2 帧级并行
    - [ ] P9.3 熵解码/重建分离
- [ ] P10 内存与缓存
    - [ ] P10.1 内存分配
    - [ ] P10.2 缓存布局
    - [ ] P10.3 位流读取
- [ ] P11 最终门禁
