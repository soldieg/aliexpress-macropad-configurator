use eframe::egui;
use std::time::{Duration, Instant};

pub mod countdown_window {
    use super::*;

    pub struct CountdownWindow {
        remaining: Duration,
        dry_run: bool,
        cancelled: std::cell::Cell<bool>,
    }

    impl CountdownWindow {
        pub fn open(dry_run: bool, _wake: impl Fn() + 'static) -> Result<Self, String> {
            Ok(Self {
                remaining: Duration::from_secs(5),
                dry_run,
                cancelled: std::cell::Cell::new(false),
            })
        }

        pub fn update(&mut self, remaining: Duration, dry_run: bool) {
            self.remaining = remaining;
            self.dry_run = dry_run;
        }

        pub fn take_cancelled(&self) -> bool {
            self.cancelled.replace(false)
        }

        pub fn render(&mut self, ctx: &egui::Context) {
            let title = if self.dry_run {
                "Simulação de desligamento"
            } else {
                "Desligamento do computador"
            };
            let title = crate::i18n::tr(title);
            let seconds = self.remaining.as_millis().div_ceil(1000);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("macro-countdown"),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([348., 150.])
                    .with_resizable(false)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop),
                |child, _| {
                    egui::CentralPanel::default().show(child, |ui| {
                        ui.heading(title);
                        ui.label(egui::RichText::new(format!("{seconds} s")).size(36.));
                        ui.add(egui::ProgressBar::new(self.remaining.as_secs_f32() / 5.));
                        if ui.button(crate::i18n::tr("Cancelar")).clicked()
                            || child.input(|input| {
                                input.viewport().close_requested()
                                    || input.key_pressed(egui::Key::Escape)
                            })
                        {
                            self.cancelled.set(true);
                            ctx.request_repaint();
                        }
                    });
                },
            );
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
}

pub mod volume_osd {
    use super::*;

    #[derive(Default)]
    pub struct VolumeOsd {
        level: u8,
        until: Option<Instant>,
    }

    impl VolumeOsd {
        pub fn show(&mut self, level: u8) {
            self.level = level.min(100);
            self.until = Some(Instant::now() + Duration::from_millis(1400));
        }

        pub fn render(&mut self, ctx: &egui::Context) {
            let Some(until) = self.until else { return };
            let now = Instant::now();
            if now >= until {
                self.until = None;
                return;
            }
            let mut viewport = egui::ViewportBuilder::default()
                .with_title(crate::i18n::tr("Volume do giratório"))
                .with_inner_size([272., 64.])
                .with_resizable(false)
                .with_decorations(false)
                .with_active(false)
                .with_taskbar(false)
                .with_mouse_passthrough(true)
                .with_window_level(egui::WindowLevel::AlwaysOnTop);
            if let Some(monitor) = ctx.input(|input| input.viewport().monitor_size) {
                viewport =
                    viewport.with_position([(monitor.x - 272.) / 2., (monitor.y - 156.).max(0.)]);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("macro-volume"),
                viewport,
                |child, _| {
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgb(25, 32, 40))
                                .inner_margin(15),
                        )
                        .show(child, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.add(
                                    egui::ProgressBar::new(f32::from(self.level) / 100.)
                                        .desired_width(157.),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{}%", self.level))
                                        .size(23.)
                                        .strong(),
                                );
                            });
                        });
                },
            );
            ctx.request_repaint_after(until - now);
        }
    }
}
