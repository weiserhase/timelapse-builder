#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use eframe::egui;
use timelapse_builder::{run, BuildOptions, Fit, Progress, Sort, Source};

const CODECS: &[&str] =
    &["libx264", "libx265", "libvpx-vp9", "h264_nvenc", "hevc_nvenc", "av1_nvenc"];
const PRESETS: &[&str] = &["ultrafast", "veryfast", "fast", "medium", "slow", "veryslow"];
const IMG_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "arw", "cr2", "cr3", "nef", "dng", "raf", "rw2", "orf", "raw",
];

enum Msg {
    Progress(Progress),
    Error(String),
}

fn nonempty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

struct App {
    inputs: Vec<PathBuf>,
    output: String,
    fps: f32,
    crf: u32,
    width: u32,
    height: u32,
    every: u32,
    fit: Fit,
    sort: Sort,
    source: Source,
    filter: String,
    sort_key: String,
    codec: String,
    preset: String,
    recursive: bool,
    reverse: bool,
    running: bool,
    total: usize,
    done: usize,
    skipped: Vec<String>,
    status: String,
    rx: Option<Receiver<Msg>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output: "timelapse.mp4".into(),
            fps: 30.0,
            crf: 18,
            width: 0,
            height: 0,
            every: 1,
            fit: Fit::Cover,
            sort: Sort::Name,
            source: Source::Auto,
            filter: String::new(),
            sort_key: String::new(),
            codec: "libx264".into(),
            preset: "medium".into(),
            recursive: false,
            reverse: false,
            running: false,
            total: 0,
            done: 0,
            skipped: Vec::new(),
            status: String::new(),
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(18, 18, 22);
        visuals.window_rounding = 8.0.into();
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        cc.egui_ctx.set_style(style);
        Self::default()
    }

    fn options(&self) -> BuildOptions {
        BuildOptions {
            inputs: self.inputs.clone(),
            output: PathBuf::from(self.output.trim()),
            fps: self.fps,
            width: (self.width != 0).then_some(self.width),
            height: (self.height != 0).then_some(self.height),
            recursive: self.recursive,
            sort: self.sort,
            reverse: self.reverse,
            every: self.every.max(1) as usize,
            limit: None,
            crf: self.crf,
            preset: self.preset.clone(),
            codec: self.codec.clone(),
            fit: self.fit,
            threads: None,
            source: self.source,
            filter: nonempty(&self.filter),
            sort_key: nonempty(&self.sort_key),
        }
    }

    fn start(&mut self) {
        let (tx, rx) = channel();
        let opts = self.options();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.rx = Some(rx);
        self.running = true;
        self.done = 0;
        self.total = 0;
        self.skipped.clear();
        self.status = "starting…".into();

        self.worker = Some(thread::spawn(move || {
            let err_tx = tx.clone();
            let res = run(&opts, &cancel, move |p| {
                let _ = tx.send(Msg::Progress(p));
            });
            if let Err(e) = res {
                let _ = err_tx.send(Msg::Error(format!("{e:#}")));
            }
        }));
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = "stopping…".into();
    }

    fn finish_worker(&mut self) {
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut messages = Vec::new();
        while let Ok(m) = rx.try_recv() {
            messages.push(m);
        }
        for m in messages {
            match m {
                Msg::Progress(Progress::Started {
                    total,
                    width,
                    height,
                    ..
                }) => {
                    self.total = total;
                    self.status = format!("encoding {total} frames at {width}×{height}…");
                }
                Msg::Progress(Progress::Advanced { done, .. }) => self.done = done,
                Msg::Progress(Progress::Skipped { path, error }) => {
                    self.skipped
                        .push(format!("{}: {error}", path.display()));
                }
                Msg::Progress(Progress::Finished {
                    encoded,
                    skipped,
                    elapsed,
                    output,
                }) => {
                    self.running = false;
                    self.rx = None;
                    let extra = if skipped > 0 {
                        format!(" · {skipped} skipped")
                    } else {
                        String::new()
                    };
                    self.status = format!(
                        "done · {encoded} frames in {elapsed:.1}s{extra} → {}",
                        output.display()
                    );
                }
                Msg::Progress(Progress::Cancelled { encoded }) => {
                    self.running = false;
                    self.rx = None;
                    self.status = format!("stopped · {encoded} frames");
                }
                Msg::Error(e) => {
                    self.running = false;
                    self.rx = None;
                    self.status = format!("error: {e}");
                }
            }
        }
        if !self.running {
            self.finish_worker();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        if self.running {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("Timelapse Builder");
            ui.label(
                egui::RichText::new("PNG · JPG · WebP · RAW  →  video")
                    .color(egui::Color32::from_gray(140)),
            );
            ui.add_space(8.0);

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add files…").clicked() {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("images", IMG_EXTS)
                            .pick_files()
                        {
                            self.inputs.extend(paths);
                        }
                    }
                    if ui.button("Add folder…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.inputs.push(dir);
                        }
                    }
                    if ui.button("Clear").clicked() {
                        self.inputs.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{} items", self.inputs.len()))
                                .color(egui::Color32::from_gray(150)),
                        );
                    });
                });
                if !self.inputs.is_empty() {
                    egui::ScrollArea::vertical()
                        .max_height(74.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for p in &self.inputs {
                                ui.label(
                                    egui::RichText::new(p.display().to_string()).monospace().small(),
                                );
                            }
                        });
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Output");
                if ui.button("Save as…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("video", &["mp4", "mkv", "mov", "webm"])
                        .set_file_name(&self.output)
                        .save_file()
                    {
                        self.output = path.display().to_string();
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut self.output).desired_width(f32::INFINITY));
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("regex on file name")
                        .desired_width(f32::INFINITY),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Order key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.sort_key)
                        .hint_text("regex; capture group is the sort key (overrides Order)")
                        .desired_width(f32::INFINITY),
                );
            });

            ui.add_space(6.0);
            egui::Grid::new("settings")
                .num_columns(4)
                .spacing(egui::vec2(14.0, 10.0))
                .show(ui, |ui| {
                    ui.label("FPS");
                    ui.add(egui::DragValue::new(&mut self.fps).range(1.0..=120.0).speed(0.5));
                    ui.label("Quality (CRF)");
                    ui.add(egui::Slider::new(&mut self.crf, 0..=51));
                    ui.end_row();

                    ui.label("Fit");
                    egui::ComboBox::from_id_salt("fit")
                        .selected_text(format!("{:?}", self.fit))
                        .show_ui(ui, |ui| {
                            for f in [Fit::Cover, Fit::Contain, Fit::Stretch] {
                                ui.selectable_value(&mut self.fit, f, format!("{f:?}"));
                            }
                        });
                    ui.label("Order");
                    egui::ComboBox::from_id_salt("sort")
                        .selected_text(format!("{:?}", self.sort))
                        .show_ui(ui, |ui| {
                            for s in [Sort::Name, Sort::Time, Sort::None] {
                                ui.selectable_value(&mut self.sort, s, format!("{s:?}"));
                            }
                        });
                    ui.end_row();

                    ui.label("Codec");
                    egui::ComboBox::from_id_salt("codec")
                        .selected_text(&self.codec)
                        .show_ui(ui, |ui| {
                            for c in CODECS {
                                ui.selectable_value(&mut self.codec, c.to_string(), *c);
                            }
                        });
                    ui.label("Preset");
                    egui::ComboBox::from_id_salt("preset")
                        .selected_text(&self.preset)
                        .show_ui(ui, |ui| {
                            for p in PRESETS {
                                ui.selectable_value(&mut self.preset, p.to_string(), *p);
                            }
                        });
                    ui.end_row();

                    ui.label("RAW source");
                    egui::ComboBox::from_id_salt("source")
                        .selected_text(format!("{:?}", self.source))
                        .show_ui(ui, |ui| {
                            for s in [Source::Auto, Source::Raw, Source::Preview] {
                                ui.selectable_value(&mut self.source, s, format!("{s:?}"));
                            }
                        })
                        .response
                        .on_hover_text(
                            "Auto: embedded preview when large enough, else demosaic.\n\
                             Raw: always demosaic (best quality).\n\
                             Preview: always use the camera JPEG (fastest).",
                        );
                    ui.end_row();

                    ui.label("Width (0 = auto)");
                    ui.add(egui::DragValue::new(&mut self.width).range(0..=16384));
                    ui.label("Height (0 = auto)");
                    ui.add(egui::DragValue::new(&mut self.height).range(0..=16384));
                    ui.end_row();

                    ui.label("Every Nth");
                    ui.add(egui::DragValue::new(&mut self.every).range(1..=100));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.recursive, "Recursive");
                        ui.checkbox(&mut self.reverse, "Reverse");
                    });
                    ui.end_row();
                });

            ui.add_space(10.0);
            if self.running {
                let stop = egui::Button::new(egui::RichText::new("Stop").size(16.0))
                    .fill(egui::Color32::from_rgb(150, 50, 50))
                    .min_size(egui::vec2(ui.available_width(), 38.0));
                if ui.add(stop).clicked() {
                    self.stop();
                }
            } else {
                let can_build = !self.inputs.is_empty();
                let build = egui::Button::new(egui::RichText::new("Build timelapse").size(16.0))
                    .min_size(egui::vec2(ui.available_width(), 38.0));
                if ui.add_enabled(can_build, build).clicked() {
                    self.start();
                }
            }

            ui.add_space(8.0);
            if self.total > 0 {
                let frac = self.done as f32 / self.total.max(1) as f32;
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(format!("{}/{}", self.done, self.total))
                        .animate(self.running),
                );
            }
            if !self.status.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&self.status).color(egui::Color32::from_gray(170)));
            }
            if !self.skipped.is_empty() {
                ui.add_space(4.0);
                egui::CollapsingHeader::new(format!("skipped ({})", self.skipped.len())).show(
                    ui,
                    |ui| {
                        for s in &self.skipped {
                            ui.label(egui::RichText::new(s).small().color(egui::Color32::from_rgb(
                                200, 160, 80,
                            )));
                        }
                    },
                );
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 600.0])
            .with_min_inner_size([460.0, 480.0])
            .with_title("Timelapse Builder"),
        ..Default::default()
    };
    eframe::run_native(
        "Timelapse Builder",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
