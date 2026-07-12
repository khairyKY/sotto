//! The settings window: a normal, resizable, OS-decorated window (light/dark
//! following the system theme) opened from the tray. Rendered as an eframe
//! *immediate* child viewport driven from the overlay app, so its state can
//! live here and borrow freely.
//!
//! Every control edits `config.toml` (persisted on change) and, for the values
//! wired live via [`Controls`], updates the shared atomics too — so hotkey,
//! activation, polish tier/threshold, and the dictionary take effect without a
//! restart. Sections without live backing (speech model) are honest read-only
//! status rather than fake UI.

use crate::config::{ActivationMode, Config, DictEntry, PolishMode};
use crate::hotkey::SUPPORTED_HOTKEYS;
use crate::Controls;
use eframe::egui::{self, vec2, Align, Color32, Layout, Margin, RichText, Stroke, TextEdit};
use std::sync::atomic::Ordering;

pub struct Settings {
    pub open: bool,
    cfg: Config,
    controls: Controls,
    launch_at_login: bool,
}

impl Settings {
    pub fn new(controls: Controls) -> Self {
        Self {
            open: false,
            cfg: Config::load_or_init().unwrap_or_default(),
            controls,
            launch_at_login: crate::startup::is_enabled(),
        }
    }

    fn persist(&self) {
        if let Err(err) = self.cfg.save() {
            tracing::error!(?err, "failed to save config from settings window");
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let th = Theme::pick(dark);

        ui.painter().rect_filled(ui.max_rect(), 0.0, th.window);
        {
            let v = &mut ui.style_mut().visuals;
            v.override_text_color = Some(th.text);
            v.selection.bg_fill = th.accent;
            v.selection.stroke = Stroke::new(1.0, th.accent);
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(Margin {
                        left: 22,
                        right: 22,
                        top: 20,
                        bottom: 20,
                    })
                    .show(ui, |ui| {
                        self.sections(ui, &th);
                    });
            });
    }

    fn sections(&mut self, ui: &mut egui::Ui, th: &Theme) {
        header(ui, th, "HOTKEY");
        card(ui, th, |ui| self.hotkey_body(ui, th));
        gap(ui);

        header(ui, th, "ACTIVATION");
        card(ui, th, |ui| self.activation_body(ui, th));
        gap(ui);

        header(ui, th, "SPEECH MODEL");
        card(ui, th, |ui| self.model_body(ui, th));
        gap(ui);

        header(ui, th, "POLISH");
        card(ui, th, |ui| self.polish_body(ui, th));
        gap(ui);

        header(ui, th, "DICTIONARY & SNIPPETS");
        card(ui, th, |ui| self.dictionary_body(ui, th));
        gap(ui);

        ui.horizontal(|ui| {
            ui.label(RichText::new("HISTORY").size(11.0).strong().color(th.header));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("Click an entry to copy it again")
                        .size(11.0)
                        .color(th.header),
                );
            });
        });
        ui.add_space(8.0);
        card(ui, th, |ui| self.history_body(ui, th));
        gap(ui);

        header(ui, th, "STARTUP");
        card(ui, th, |ui| self.startup_body(ui, th));
    }

    fn hotkey_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Dictation key").size(13.0).color(th.text));
                ui.label(
                    RichText::new("Hold to talk — release to transcribe")
                        .size(12.0)
                        .color(th.secondary),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let cur = self.controls.hotkey_idx.load(Ordering::Relaxed);
                let mut idx = cur;
                egui::ComboBox::from_id_salt("hotkey_pick")
                    .selected_text(SUPPORTED_HOTKEYS[cur.min(SUPPORTED_HOTKEYS.len() - 1)].0)
                    .show_ui(ui, |ui| {
                        for (i, (disp, _, _)) in SUPPORTED_HOTKEYS.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, *disp);
                        }
                    });
                if idx != cur {
                    self.controls.hotkey_idx.store(idx, Ordering::Relaxed);
                    self.cfg.hotkey = SUPPORTED_HOTKEYS[idx].1.to_string();
                    self.persist();
                }
            });
        });
    }

    fn activation_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Trigger").size(13.0).color(th.text));
                ui.label(
                    RichText::new("Hold the key, or press once to start & stop")
                        .size(12.0)
                        .color(th.secondary),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut mode = ActivationMode::from_u8(self.controls.activation.load(Ordering::Relaxed));
                if segmented(
                    ui,
                    th,
                    &mut mode,
                    &[(ActivationMode::Hold, "Hold"), (ActivationMode::Toggle, "Toggle")],
                ) {
                    self.controls.activation.store(mode.as_u8(), Ordering::Relaxed);
                    self.cfg.activation_mode = mode;
                    self.persist();
                }
            });
        });
    }

    fn model_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let installed = crate::config::model_dir().exists();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Parakeet v3 · English").size(13.0).color(th.text));
                ui.label(RichText::new("NVIDIA · int8 quantized").size(12.0).color(th.secondary));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(mb) = dir_size_mb(crate::config::model_dir()) {
                    ui.label(RichText::new(format!("{mb} MB")).size(12.0).color(th.secondary));
                    ui.add_space(10.0);
                }
                pill(
                    ui,
                    th,
                    if installed { "Installed" } else { "Not installed" },
                );
            });
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Downloading and switching models is coming later — this build uses the installed Parakeet model.",
            )
            .size(11.5)
            .color(th.secondary),
        );
    }

    fn polish_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cleanup").size(13.0).color(th.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut mode = PolishMode::from_u8(self.controls.polish_mode.load(Ordering::Relaxed));
                if segmented(
                    ui,
                    th,
                    &mut mode,
                    &[
                        (PolishMode::Off, "Off"),
                        (PolishMode::Rules, "Rules"),
                        (PolishMode::Ai, "AI"),
                    ],
                ) {
                    self.controls.polish_mode.store(mode.as_u8(), Ordering::Relaxed);
                    self.cfg.polish.mode = mode;
                    self.persist();
                }
            });
        });

        ui.add_space(10.0);
        divider(ui, th);
        ui.add_space(10.0);

        let ai = PolishMode::from_u8(self.controls.polish_mode.load(Ordering::Relaxed)) == PolishMode::Ai;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Use AI past").size(13.0).color(th.text));
                ui.label(
                    RichText::new("shorter clips stay on instant rules")
                        .size(12.0)
                        .color(th.secondary),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut words = self.controls.ai_min_words.load(Ordering::Relaxed) as u32;
                let resp = ui.add_enabled(ai, egui::Slider::new(&mut words, 0..=60).suffix(" words"));
                if resp.changed() {
                    self.controls.ai_min_words.store(words as usize, Ordering::Relaxed);
                    self.cfg.polish.ai_min_words = words as usize;
                    self.persist();
                }
            });
        });
    }

    fn dictionary_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let mut changed = false;
        let mut remove = None;

        for (i, entry) in self.cfg.dictionary.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui
                    .add(TextEdit::singleline(&mut entry.spoken).hint_text("spoken").desired_width(150.0))
                    .changed()
                {
                    changed = true;
                }
                ui.label(RichText::new("→").color(th.secondary));
                if ui
                    .add(
                        TextEdit::singleline(&mut entry.replacement)
                            .hint_text("replacement")
                            .desired_width(150.0),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            });
            ui.add_space(6.0);
        }

        if let Some(i) = remove {
            self.cfg.dictionary.remove(i);
            changed = true;
        }

        ui.add_space(4.0);
        if ui
            .add(egui::Button::new(RichText::new("+ Add entry").size(12.5).color(th.accent)).frame(false))
            .clicked()
        {
            self.cfg.dictionary.push(DictEntry::default());
            changed = true;
        }

        if changed {
            // Sync the live dictionary (drop blank rows) + persist.
            *self.controls.dictionary.lock().unwrap() = self
                .cfg
                .dictionary
                .iter()
                .filter(|e| !e.spoken.trim().is_empty())
                .map(|e| (e.spoken.clone(), e.replacement.clone()))
                .collect();
            self.persist();
        }
    }

    fn history_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let entries = self.controls.history.snapshot();
        if entries.is_empty() {
            ui.label(
                RichText::new("Nothing dictated yet this session")
                    .size(12.5)
                    .color(th.secondary),
            );
            return;
        }

        const ROW_H: f32 = 26.0;
        for (i, entry) in entries.iter().enumerate() {
            let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), egui::Sense::click());

            if row.hovered() {
                ui.painter().rect_filled(rect, 4.0, th.divider);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if row.clicked() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(entry.text.clone());
                    tracing::info!("re-copied history entry to clipboard");
                }
            }

            // Content painted in a child Ui on top of the (possibly highlighted)
            // background allocated above.
            let mut content = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
            content.add_sized(
                [58.0, ROW_H],
                egui::Label::new(RichText::new(&entry.time).monospace().size(11.0).color(th.secondary)),
            );
            content.add_space(4.0);
            content.label(RichText::new(&entry.text).size(12.5).color(th.text));
            content.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("⧉").size(12.0).color(th.secondary));
            });

            if i + 1 < entries.len() {
                ui.add_space(4.0);
                divider(ui, th);
                ui.add_space(4.0);
            }
        }
    }

    fn startup_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Launch Sotto at login").size(13.0).color(th.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.checkbox(&mut self.launch_at_login, "").changed() {
                    if let Err(err) = crate::startup::set_enabled(self.launch_at_login) {
                        tracing::error!(?err, "failed to set launch-at-login");
                        self.launch_at_login = crate::startup::is_enabled(); // reflect reality
                    }
                }
            });
        });
    }
}

// ── styling helpers ──────────────────────────────────────────────────────

struct Theme {
    window: Color32,
    card: Color32,
    text: Color32,
    secondary: Color32,
    header: Color32,
    divider: Color32,
    border: Color32,
    accent: Color32,
    input: Color32,
}

impl Theme {
    fn pick(dark: bool) -> Self {
        if dark {
            Theme {
                window: Color32::from_rgb(0x20, 0x1F, 0x22),
                card: Color32::from_rgb(0x26, 0x25, 0x2A),
                text: Color32::from_rgb(0xE8, 0xE5, 0xDF),
                secondary: Color32::from_rgb(0x92, 0x8E, 0x85),
                header: Color32::from_rgb(0x7C, 0x78, 0x6F),
                divider: Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                border: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
                accent: Color32::from_rgb(0x4F, 0xCF, 0xDB),
                input: Color32::from_rgb(0x17, 0x16, 0x1A),
            }
        } else {
            Theme {
                window: Color32::from_rgb(0xFB, 0xFA, 0xF8),
                card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                text: Color32::from_rgb(0x23, 0x22, 0x1F),
                secondary: Color32::from_rgb(0x6F, 0x6C, 0x64),
                header: Color32::from_rgb(0x8A, 0x85, 0x7B),
                divider: Color32::from_rgba_unmultiplied(0, 0, 0, 15),
                border: Color32::from_rgba_unmultiplied(0, 0, 0, 28),
                accent: Color32::from_rgb(0x1B, 0x93, 0xA1),
                input: Color32::from_rgb(0xFB, 0xFA, 0xF8),
            }
        }
    }
}

fn header(ui: &mut egui::Ui, th: &Theme, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(th.header));
    ui.add_space(8.0);
}

fn gap(ui: &mut egui::Ui) {
    ui.add_space(22.0);
}

fn card(ui: &mut egui::Ui, th: &Theme, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(th.card)
        .stroke(Stroke::new(1.0, th.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn divider(ui: &mut egui::Ui, th: &Theme) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, th.divider);
}

fn pill(ui: &mut egui::Ui, th: &Theme, text: &str) {
    let bg = Color32::from_rgba_unmultiplied(th.accent.r(), th.accent.g(), th.accent.b(), 36);
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(20)
        .inner_margin(Margin {
            left: 9,
            right: 9,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(th.accent));
        });
}

/// A segmented control: a row of selectable options in an inset track. Returns
/// true if the selection changed.
fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    th: &Theme,
    current: &mut T,
    opts: &[(T, &str)],
) -> bool {
    let mut changed = false;
    egui::Frame::NONE
        .fill(th.input)
        .corner_radius(8)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (val, label) in opts {
                    if ui.selectable_value(current, *val, *label).changed() {
                        changed = true;
                    }
                }
            });
        });
    changed
}

fn dir_size_mb(dir: std::path::PathBuf) -> Option<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Some(total / 1_000_000)
}
