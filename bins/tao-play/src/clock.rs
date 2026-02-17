//! 媒体时钟模块.
//!
//! 提供 A/V 同步所需的时间参考. 以音频回调时钟为主时钟.
//!
//! Seek 安全: `seek_pending` 为 true 时, `update_audio_pts` 被忽略
//! (防止旧音频数据覆盖 seek 目标). Player 线程通过 `confirm_seek`
//! 在首帧解码完成后显式解冻时钟.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Instant;

/// 媒体时钟 (线程安全)
#[derive(Clone)]
pub struct MediaClock {
    inner: Arc<ClockInner>,
}

struct ClockInner {
    /// 时钟创建时间 (用于音频未启动前的系统时钟回退)
    start_time: Instant,
    /// 音频 PTS (微秒)
    audio_pts_us: AtomicI64,
    /// 上次音频 PTS 更新的系统时间
    audio_pts_update_time: std::sync::Mutex<Option<Instant>>,
    /// 是否已暂停
    paused: AtomicBool,
    /// 暂停时的时间偏移
    pause_offset_us: AtomicI64,
    /// Seek 后冻结时钟, 由 player 线程显式解冻
    seek_pending: AtomicBool,
}

impl MediaClock {
    /// 创建新时钟
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ClockInner {
                start_time: Instant::now(),
                audio_pts_us: AtomicI64::new(0),
                audio_pts_update_time: std::sync::Mutex::new(None),
                paused: AtomicBool::new(false),
                pause_offset_us: AtomicI64::new(0),
                seek_pending: AtomicBool::new(false),
            }),
        }
    }

    /// 更新音频 PTS (由音频回调线程调用)
    ///
    /// Seek 期间 (`seek_pending=true`) 忽略更新, 防止旧音频数据
    /// 覆盖 seek 目标导致时钟不重置.
    pub fn update_audio_pts(&self, pts_us: i64) {
        if self.inner.seek_pending.load(Ordering::Acquire) {
            return;
        }
        self.inner.audio_pts_us.store(pts_us, Ordering::Relaxed);
        *self.inner.audio_pts_update_time.lock().unwrap() = Some(Instant::now());
    }

    /// Seek 重置: 冻结时钟到目标时间
    ///
    /// 时钟被冻结直到 `confirm_seek()` 被调用 (player 线程在
    /// 首帧解码完成后调用), 防止:
    /// - 旧音频回调覆盖目标 PTS (竞态)
    /// - 时钟提前推进导致 seek 后加速播放
    pub fn seek_reset(&self, target_us: i64) {
        self.inner.seek_pending.store(true, Ordering::Release);
        self.inner.audio_pts_us.store(target_us, Ordering::Relaxed);
        *self.inner.audio_pts_update_time.lock().unwrap() = None;
    }

    /// 确认 seek 完成: 解冻时钟, 开始从 seek 目标推进
    ///
    /// 由 player 线程在 seek 后首帧解码完成时调用.
    /// 设置 `audio_pts_update_time` 使时钟从当前值开始正常推进,
    /// 避免回退到系统时钟.
    pub fn confirm_seek(&self) {
        *self.inner.audio_pts_update_time.lock().unwrap() = Some(Instant::now());
        self.inner.seek_pending.store(false, Ordering::Release);
    }

    /// 获取当前播放时间 (微秒)
    ///
    /// 四种模式:
    /// - 暂停中: 返回冻结的音频 PTS
    /// - Seek 冻结: 返回目标 PTS (不推进)
    /// - 正常播放: 音频 PTS + 经过时间
    /// - 初始启动: 系统时钟兜底
    pub fn current_time_us(&self) -> i64 {
        if self.inner.paused.load(Ordering::Relaxed) {
            return self.inner.audio_pts_us.load(Ordering::Relaxed);
        }

        if self.inner.seek_pending.load(Ordering::Acquire) {
            return self.inner.audio_pts_us.load(Ordering::Relaxed);
        }

        let base_pts = self.inner.audio_pts_us.load(Ordering::Relaxed);
        let guard = self.inner.audio_pts_update_time.lock().unwrap();
        if let Some(update_time) = *guard {
            // 音频已启动: 使用音频 PTS + 上次更新后的经过时间
            let elapsed = update_time.elapsed().as_micros() as i64;
            base_pts + elapsed
        } else {
            // 初始播放: 回退到系统时钟, 防止所有帧以 delay>0 堆积快速渲染
            self.inner.start_time.elapsed().as_micros() as i64
        }
    }

    /// 切换暂停状态
    pub fn toggle_pause(&self) {
        let was_paused = self.inner.paused.load(Ordering::Relaxed);
        self.inner.paused.store(!was_paused, Ordering::Relaxed);
    }

    /// 是否已暂停
    pub fn is_paused(&self) -> bool {
        self.inner.paused.load(Ordering::Relaxed)
    }

    /// 设置暂停
    #[allow(dead_code)]
    pub fn set_paused(&self, paused: bool) {
        self.inner.paused.store(paused, Ordering::Relaxed);
    }

    /// 获取暂停偏移量
    #[allow(dead_code)]
    pub fn pause_offset_us(&self) -> i64 {
        self.inner.pause_offset_us.load(Ordering::Relaxed)
    }
}
