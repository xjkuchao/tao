//! 媒体时钟模块.
//!
//! 提供 A/V 同步所需的时间参考. 以音频回调时钟为主时钟.

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
            }),
        }
    }

    /// 更新音频 PTS (由音频回调线程调用)
    pub fn update_audio_pts(&self, pts_us: i64) {
        self.inner.audio_pts_us.store(pts_us, Ordering::Relaxed);
        *self.inner.audio_pts_update_time.lock().unwrap() = Some(Instant::now());
    }

    /// 获取当前播放时间 (微秒)
    ///
    /// 优先使用音频 PTS + 经过时间;
    /// 音频尚未启动时, 回退到系统时钟 (避免开头帧堆积加速).
    pub fn current_time_us(&self) -> i64 {
        if self.inner.paused.load(Ordering::Relaxed) {
            return self.inner.audio_pts_us.load(Ordering::Relaxed);
        }

        let base_pts = self.inner.audio_pts_us.load(Ordering::Relaxed);
        let guard = self.inner.audio_pts_update_time.lock().unwrap();
        if let Some(update_time) = *guard {
            // 音频已启动: 使用音频 PTS + 上次更新后的经过时间
            let elapsed = update_time.elapsed().as_micros() as i64;
            base_pts + elapsed
        } else {
            // 音频未启动: 回退到系统时钟, 防止所有帧以 delay>0 堆积快速渲染
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
