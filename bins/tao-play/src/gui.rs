use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};

use crate::player::{PlayerCommand, PlayerStatus, VideoFrame};

pub struct PlayerApp {
    frame_rx: Receiver<VideoFrame>,
    status_rx: Receiver<PlayerStatus>,
    command_tx: Sender<PlayerCommand>,

    current_frame: Option<egui::TextureHandle>,
    last_frame_data: Option<VideoFrame>,

    volume: f32,
    is_paused: bool, // Display only
    current_time: f64,
    total_duration: f64,

    osd_text: String,
    osd_timer: f32,
}

impl PlayerApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        frame_rx: Receiver<VideoFrame>,
        status_rx: Receiver<PlayerStatus>,
        command_tx: Sender<PlayerCommand>,
        initial_volume: f32,
    ) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = 8.0.into();
        cc.egui_ctx.set_visuals(visuals);

        Self {
            frame_rx,
            status_rx,
            command_tx,
            current_frame: None,
            last_frame_data: None,
            volume: initial_volume,
            is_paused: false,
            current_time: 0.0,
            total_duration: 0.0,
            osd_text: String::new(),
            osd_timer: 0.0,
        }
    }
}

impl eframe::App for PlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Receive frames
        while let Ok(frame) = self.frame_rx.try_recv() {
            self.last_frame_data = Some(frame);
        }

        // Update texture if we have new frame data
        if let Some(frame_data) = &self.last_frame_data {
            let color_image = egui::ColorImage::from_rgb(
                [frame_data.width as usize, frame_data.height as usize],
                &frame_data.data,
            );

            self.current_frame =
                Some(ctx.load_texture("video_frame", color_image, egui::TextureOptions::LINEAR));
        }
        // Optimization: clear last_frame_data to prevent constant re-upload
        self.last_frame_data = None;

        // Receive status updates
        while let Ok(status) = self.status_rx.try_recv() {
            match status {
                PlayerStatus::Time(t, d) => {
                    self.current_time = t;
                    self.total_duration = d;
                }
                PlayerStatus::Paused(p) => self.is_paused = p,
                PlayerStatus::Volume(v) => self.volume = v,
                PlayerStatus::End => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                PlayerStatus::Error(e) => {
                    self.osd_text = format!("Error: {}", e);
                    self.osd_timer = 5.0;
                }
            }
        }

        // Handle Input
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            let _ = self.command_tx.send(PlayerCommand::TogglePause);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            let _ = self.command_tx.send(PlayerCommand::Seek(10.0));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            let _ = self.command_tx.send(PlayerCommand::Seek(-10.0));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            let _ = self.command_tx.send(PlayerCommand::VolumeUp);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            let _ = self.command_tx.send(PlayerCommand::VolumeDown);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::M)) {
            let _ = self.command_tx.send(PlayerCommand::ToggleMute);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            let _ = self.command_tx.send(PlayerCommand::Stop);
        }

        // Render UI
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                // Draw video
                if let Some(texture) = &self.current_frame {
                    let texture_size = texture.size_vec2();
                    let available_size = ui.available_size();

                    // Calculate final size: use original size if it fits, otherwise scale down
                    let final_size = if texture_size.x <= available_size.x
                        && texture_size.y <= available_size.y
                    {
                        // Video fits in window, use original size
                        texture_size
                    } else {
                        // Video is larger than window, scale down maintaining aspect ratio
                        let texture_aspect = texture_size.x / texture_size.y;
                        let available_aspect = available_size.x / available_size.y;

                        if texture_aspect > available_aspect {
                            // Video is wider than available space
                            egui::Vec2::new(available_size.x, available_size.x / texture_aspect)
                        } else {
                            // Video is taller than available space
                            egui::Vec2::new(available_size.y * texture_aspect, available_size.y)
                        }
                    };

                    // Center the video
                    let offset = (available_size - final_size) * 0.5;
                    let image_rect =
                        egui::Rect::from_min_size(ui.min_rect().min + offset, final_size);

                    // Draw the video texture
                    ui.painter().image(
                        texture.id(),
                        image_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }

                // Draw OSD / Controls overlay
                let max_rect = ui.max_rect();

                // Progress Bar
                if self.total_duration > 0.0 {
                    let progress = (self.current_time / self.total_duration).clamp(0.0, 1.0);
                    let bar_height = 4.0;
                    let bar_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(10.0, max_rect.bottom() - 20.0),
                        egui::Vec2::new(max_rect.width() - 20.0, bar_height),
                    );

                    // Background
                    ui.painter()
                        .rect_filled(bar_rect, 2.0, egui::Color32::from_gray(50));

                    // Fill
                    let fill_rect = egui::Rect::from_min_size(
                        bar_rect.min,
                        egui::Vec2::new(bar_rect.width() * progress as f32, bar_height),
                    );
                    ui.painter()
                        .rect_filled(fill_rect, 2.0, egui::Color32::from_rgb(0, 119, 204));
                }

                // OSD Text
                if self.osd_timer > 0.0 {
                    self.osd_timer -= ctx.input(|i| i.stable_dt);
                    ui.painter().text(
                        egui::Pos2::new(20.0, 20.0),
                        egui::Align2::LEFT_TOP,
                        &self.osd_text,
                        egui::FontId::proportional(20.0),
                        egui::Color32::WHITE,
                    );
                }
            });

        // Request repaint continuously to ensure smooth video playback
        ctx.request_repaint();
    }
}
