//! SDL2 视频渲染和事件循环.
//!
//! 实现 ffplay 风格的 video_refresh 状态机, 在渲染线程精确控制帧显示时机.
//! 使用 SDL2 YUV 纹理进行硬件加速渲染, GPU 做色彩空间转换.

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureAccess, TextureCreator};
use sdl2::video::{Window, WindowContext};
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::clock::MediaClock;
use crate::player::{PlayerCommand, PlayerStatus, VideoFrame};

// ── ffplay 同步常量 ──────────────────────────────────────────────────────

/// 最小同步阈值 (秒) - ffplay: AV_SYNC_THRESHOLD_MIN
const AV_SYNC_THRESHOLD_MIN: f64 = 0.04;
/// 最大同步阈值 (秒) - ffplay: AV_SYNC_THRESHOLD_MAX
const AV_SYNC_THRESHOLD_MAX: f64 = 0.1;
/// 帧重复阈值 (秒) - ffplay: AV_SYNC_FRAMEDUP_THRESHOLD
const AV_SYNC_FRAMEDUP_THRESHOLD: f64 = 0.1;
/// 不同步阈值 (秒) - ffplay: AV_NOSYNC_THRESHOLD
const AV_NOSYNC_THRESHOLD: f64 = 10.0;
/// 默认刷新率 (秒) - ffplay: REFRESH_RATE
const REFRESH_RATE: f64 = 0.01;

// ── 挂钟时间 ─────────────────────────────────────────────────────────────

static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// 获取挂钟时间 (秒), 对应 ffplay 的 `av_gettime_relative() / 1000000.0`
fn wall_clock_sec() -> f64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

// ── 视频显示状态 ─────────────────────────────────────────────────────────

/// 视频显示状态 (对应 ffplay VideoState 中与渲染相关的字段)
struct VideoDisplayState<'a> {
    /// 帧定时器: 当前帧应显示的挂钟时间 (秒)
    frame_timer: f64,
    /// 待显示帧队列 (从 player 线程接收)
    frame_queue: VecDeque<VideoFrame>,
    /// 最后显示帧的 PTS (秒)
    last_pts: f64,
    /// 是否需要强制刷新
    force_refresh: bool,
    /// 单步模式 (显示一帧后暂停)
    step: bool,
    /// 帧丢弃统计
    frame_drops_late: u64,
    /// SDL2 纹理
    texture: Option<Texture<'a>>,
    /// 纹理尺寸
    tex_width: u32,
    tex_height: u32,
    /// 全屏状态
    is_fullscreen: bool,
    /// Seek 后等待新帧显示 (暂停状态下)
    seek_frame_pending: bool,
}

impl<'a> VideoDisplayState<'a> {
    fn new() -> Self {
        Self {
            frame_timer: 0.0,
            frame_queue: VecDeque::with_capacity(8),
            last_pts: f64::NAN,
            force_refresh: false,
            step: false,
            frame_drops_late: 0,
            texture: None,
            tex_width: 0,
            tex_height: 0,
            is_fullscreen: false,
            seek_frame_pending: false,
        }
    }
}

/// 格式化 PTS 用于日志输出, NaN 显示为 "N/A"
fn fmt_pts(pts: f64) -> String {
    if pts.is_nan() {
        "N/A".to_string()
    } else {
        format!("{:.3}s", pts)
    }
}

// ── 同步算法 ─────────────────────────────────────────────────────────────

/// 计算两帧之间的持续时间 (秒), 对应 ffplay 的 `vp_duration`
fn frame_duration(last_pts: f64, current_pts: f64) -> f64 {
    if last_pts.is_nan() {
        return 0.0;
    }
    let dur = current_pts - last_pts;
    if dur <= 0.0 || dur > 10.0 { 0.0 } else { dur }
}

/// 计算目标延迟 (秒), 完全对齐 ffplay 的 `compute_target_delay`
fn compute_target_delay(delay: f64, video_pts: f64, clock: &MediaClock) -> f64 {
    let audio_time = clock.current_time_us() as f64 / 1_000_000.0;
    let diff = video_pts - audio_time;

    let sync_threshold = delay.clamp(AV_SYNC_THRESHOLD_MIN, AV_SYNC_THRESHOLD_MAX);

    if diff.is_nan() || diff.abs() >= AV_NOSYNC_THRESHOLD {
        return delay;
    }

    if diff <= -sync_threshold {
        (delay + diff).max(0.0)
    } else if diff >= sync_threshold && delay > AV_SYNC_FRAMEDUP_THRESHOLD {
        delay + diff
    } else if diff >= sync_threshold {
        2.0 * delay
    } else {
        delay
    }
}

/// 视频刷新 (对齐 ffplay video_refresh)
///
/// 返回 `(remaining_time, step_completed)`:
/// - `remaining_time`: 距下次刷新的等待时间 (秒)
/// - `step_completed`: 是否在本次刷新中完成了单步 (用于事件循环重新暂停)
fn video_refresh<'a>(
    state: &mut VideoDisplayState<'a>,
    clock: &MediaClock,
    canvas: &mut Canvas<Window>,
    texture_creator: &'a TextureCreator<WindowContext>,
    paused: bool,
) -> (f64, bool) {
    let mut remaining_time = REFRESH_RATE;
    let mut step_completed = false;

    if state.frame_queue.is_empty() {
        if state.force_refresh {
            render_current_texture(state, canvas);
            state.force_refresh = false;
        }
        return (remaining_time, false);
    }

    // 暂停时只重绘, 不推进帧
    if paused {
        if state.seek_frame_pending && !state.frame_queue.is_empty() {
            let front_pts = state.frame_queue.front().map(|f| f.pts).unwrap_or(0.0);
            log::info!(
                "[GUI] Seek 帧显示: PTS={:.3}s, 队列={}, 暂停=true",
                front_pts,
                state.frame_queue.len()
            );
            // Seek 后收到新帧: 显示并停留 (对齐 ffplay 暂停 seek)
            upload_front_frame(state, texture_creator);
            render_current_texture(state, canvas);
            state.frame_queue.pop_front();
            state.seek_frame_pending = false;
            state.force_refresh = false;
        } else if state.force_refresh {
            render_current_texture(state, canvas);
            state.force_refresh = false;
        }
        return (remaining_time, false);
    }

    // ── retry 循环: 对应 ffplay video_refresh 中的 retry 标签 ──
    loop {
        if state.frame_queue.is_empty() {
            break;
        }

        let vp_pts = state.frame_queue[0].pts;
        let last_duration = frame_duration(state.last_pts, vp_pts);
        let delay = compute_target_delay(last_duration, vp_pts, clock);

        let time = wall_clock_sec();

        // 初始化 frame_timer
        if state.frame_timer == 0.0 {
            state.frame_timer = time;
        }

        // 还没到显示时间
        if time < state.frame_timer + delay {
            remaining_time = (state.frame_timer + delay - time).min(remaining_time);
            break;
        }

        // 推进 frame_timer
        state.frame_timer += delay;

        // 时钟漂移补偿
        if delay > 0.0 && time - state.frame_timer > AV_SYNC_THRESHOLD_MAX {
            state.frame_timer = time;
        }

        // 迟到帧丢弃
        if state.frame_queue.len() > 1 {
            let next_pts = state.frame_queue[1].pts;
            let duration = frame_duration(vp_pts, next_pts);
            if !state.step && time > state.frame_timer + duration {
                state.frame_drops_late += 1;
                state.last_pts = vp_pts;
                state.frame_queue.pop_front();
                continue;
            }
        }

        // 该帧应该显示
        state.last_pts = vp_pts;
        state.force_refresh = true;

        if state.step {
            state.step = false;
            step_completed = true;
        }

        break;
    }

    // ── display: 刷新画面 ──
    if state.force_refresh && !state.frame_queue.is_empty() {
        upload_front_frame(state, texture_creator);
        render_current_texture(state, canvas);
        state.frame_queue.pop_front();
        state.force_refresh = false;
    }

    (remaining_time, step_completed)
}

// ── 渲染辅助 ─────────────────────────────────────────────────────────────

/// 将队列头部帧数据上传到 GPU 纹理
fn upload_front_frame<'a>(
    state: &mut VideoDisplayState<'a>,
    texture_creator: &'a TextureCreator<WindowContext>,
) {
    let frame = match state.frame_queue.front() {
        Some(f) => f,
        None => return,
    };

    // 纹理尺寸变化时重新创建
    if state.texture.is_none() || frame.width != state.tex_width || frame.height != state.tex_height
    {
        state.tex_width = frame.width;
        state.tex_height = frame.height;
        state.texture = texture_creator
            .create_texture(
                PixelFormatEnum::IYUV,
                TextureAccess::Streaming,
                state.tex_width,
                state.tex_height,
            )
            .ok();
    }

    if let Some(tex) = state.texture.as_mut() {
        let _ = tex.update_yuv(
            None,
            &frame.y_data,
            frame.y_stride,
            &frame.u_data,
            frame.u_stride,
            &frame.v_data,
            frame.v_stride,
        );
    }
}

/// 渲染已上传纹理到 canvas (保持宽高比)
fn render_current_texture(state: &VideoDisplayState, canvas: &mut Canvas<Window>) {
    if let Some(tex) = state.texture.as_ref() {
        canvas.clear();
        let dst = calculate_display_rect(canvas, state.tex_width, state.tex_height);
        let _ = canvas.copy(tex, None, Some(dst));
        canvas.present();
    }
}

/// 计算保持宽高比的居中显示矩形 (对齐 ffplay calculate_display_rect)
fn calculate_display_rect(canvas: &Canvas<Window>, pic_width: u32, pic_height: u32) -> Rect {
    let (scr_w, scr_h) = canvas.output_size().unwrap_or((pic_width, pic_height));
    if pic_width == 0 || pic_height == 0 || scr_w == 0 || scr_h == 0 {
        return Rect::new(0, 0, scr_w, scr_h);
    }

    // 先以窗口高度为基准计算宽度
    let mut height = scr_h as i64;
    let mut width = (height * pic_width as i64 / pic_height as i64) & !1;

    if width > scr_w as i64 {
        width = scr_w as i64;
        height = (width * pic_height as i64 / pic_width as i64) & !1;
    }

    let x = ((scr_w as i64 - width) / 2) as i32;
    let y = ((scr_h as i64 - height) / 2) as i32;

    Rect::new(x, y, width.max(1) as u32, height.max(1) as u32)
}

/// 切换全屏 (对齐 ffplay toggle_full_screen)
fn toggle_fullscreen(state: &mut VideoDisplayState, canvas: &mut Canvas<Window>) {
    state.is_fullscreen = !state.is_fullscreen;
    let flag = if state.is_fullscreen {
        sdl2::video::FullscreenType::Desktop
    } else {
        sdl2::video::FullscreenType::Off
    };
    let _ = canvas.window_mut().set_fullscreen(flag);
    state.force_refresh = true;
}

/// 返回当前播放模式的中文描述
fn play_mode_str(paused: bool, step: bool) -> &'static str {
    if step {
        "单步"
    } else if paused {
        "暂停"
    } else {
        "播放"
    }
}

// ── 事件循环 ─────────────────────────────────────────────────────────────

/// 运行 SDL2 事件循环 (在主线程)
///
/// 实现 ffplay 风格的 `refresh_loop_wait_event`:
/// - 处理 SDL 事件
/// - 调用 `video_refresh` 决定帧显示时机
/// - 按 `remaining_time` 精确休眠
pub fn run_event_loop(
    mut canvas: Canvas<Window>,
    frame_rx: Receiver<VideoFrame>,
    status_rx: Receiver<PlayerStatus>,
    command_tx: std::sync::mpsc::Sender<PlayerCommand>,
    clock: MediaClock,
    hold: bool,
) -> Result<(), String> {
    let texture_creator = canvas.texture_creator();
    let mut state = VideoDisplayState::new();
    let mut paused = false;
    let mut eof = false;
    // EOF 后是否已进入 hold 停留状态
    let mut holding = false;

    let sdl_context = canvas.window().subsystem().sdl();
    let mut event_pump = sdl_context.event_pump()?;

    'running: loop {
        // 1. 处理 SDL2 事件 (对齐 ffplay event_loop)
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => {
                    let _ = command_tx.send(PlayerCommand::Stop);
                    break 'running;
                }
                Event::KeyDown {
                    keycode: Some(key), ..
                } => match key {
                    Keycode::Escape | Keycode::Q => {
                        let _ = command_tx.send(PlayerCommand::Stop);
                        break 'running;
                    }
                    Keycode::Space | Keycode::P => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] Space/P (暂停/恢复), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        let _ = command_tx.send(PlayerCommand::TogglePause);
                    }
                    Keycode::F => {
                        toggle_fullscreen(&mut state, &mut canvas);
                    }
                    Keycode::S => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] S (单步), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        state.step = true;
                        let _ = command_tx.send(PlayerCommand::StepFrame);
                    }
                    Keycode::Right => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] Right (+10s), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        let _ = command_tx.send(PlayerCommand::Seek(10.0));
                    }
                    Keycode::Left => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] Left (-10s), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        let _ = command_tx.send(PlayerCommand::Seek(-10.0));
                    }
                    Keycode::Up => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] Up (+60s), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        let _ = command_tx.send(PlayerCommand::Seek(60.0));
                    }
                    Keycode::Down => {
                        let mode = play_mode_str(paused, state.step);
                        log::info!(
                            "[按键] Down (-60s), 当前={}, 帧队列={}, 最近PTS={}",
                            mode,
                            state.frame_queue.len(),
                            fmt_pts(state.last_pts)
                        );
                        let _ = command_tx.send(PlayerCommand::Seek(-60.0));
                    }
                    Keycode::Num9 | Keycode::KpDivide => {
                        let _ = command_tx.send(PlayerCommand::VolumeDown);
                    }
                    Keycode::Num0 | Keycode::KpMultiply => {
                        let _ = command_tx.send(PlayerCommand::VolumeUp);
                    }
                    Keycode::M => {
                        let _ = command_tx.send(PlayerCommand::ToggleMute);
                    }
                    _ => {}
                },
                Event::Window { win_event, .. } => {
                    use sdl2::event::WindowEvent;
                    if matches!(
                        win_event,
                        WindowEvent::SizeChanged(..) | WindowEvent::Exposed
                    ) {
                        state.force_refresh = true;
                    }
                }
                Event::MouseButtonDown {
                    mouse_btn, clicks, ..
                } => {
                    use sdl2::mouse::MouseButton;
                    if mouse_btn == MouseButton::Left && clicks >= 2 {
                        toggle_fullscreen(&mut state, &mut canvas);
                    }
                }
                _ => {}
            }
        }

        // 2. 接收播放状态更新
        while let Ok(status) = status_rx.try_recv() {
            match status {
                PlayerStatus::End => {
                    eof = true;
                    log::info!("收到播放结束信号，等待帧队列排空");
                }
                PlayerStatus::Paused(p) => paused = p,
                PlayerStatus::Seeked => {
                    let old_queue_len = state.frame_queue.len();
                    // Seek 完成: 清空帧队列和重置 frame_timer
                    state.frame_queue.clear();
                    state.frame_timer = 0.0;
                    state.last_pts = f64::NAN;
                    state.force_refresh = true;
                    state.seek_frame_pending = true;
                    // Seek 后重置 EOF/hold 状态 (player 线程已恢复)
                    if eof || holding {
                        eof = false;
                        holding = false;
                        paused = false;
                    }
                    log::info!(
                        "[GUI] Seek 状态: 清空帧队列 (原{}帧), 等待新帧",
                        old_queue_len
                    );
                }
                _ => {}
            }
        }

        // 3. 从 player 线程接收已解码帧
        while let Ok(frame) = frame_rx.try_recv() {
            state.frame_queue.push_back(frame);
        }

        // 4. 视频刷新: 决定帧显示时机
        let (remaining_time, step_completed) =
            video_refresh(&mut state, &clock, &mut canvas, &texture_creator, paused);

        // 检查是否播放完毕
        if eof && state.frame_queue.is_empty() && !holding {
            if hold {
                // --hold 模式: 停留在最后一帧, 等待用户退出
                holding = true;
                paused = true;
                state.force_refresh = true;
                log::info!("播放完成, --hold 模式: 停留在最后一帧");
            } else {
                log::info!("播放完成, 退出");
                break 'running;
            }
        }

        // 5. 单步完成后重新暂停 (对齐 ffplay: if is->step && !is->paused toggle_pause)
        if step_completed {
            let _ = command_tx.send(PlayerCommand::TogglePause);
        }

        // 6. 精确休眠 (对齐 ffplay av_usleep)
        if remaining_time > 0.0 {
            let sleep_us = (remaining_time * 1_000_000.0) as u64;
            std::thread::sleep(std::time::Duration::from_micros(sleep_us));
        }
    }

    if state.frame_drops_late > 0 {
        log::info!("渲染线程丢弃帧数: {}", state.frame_drops_late);
    }

    Ok(())
}
