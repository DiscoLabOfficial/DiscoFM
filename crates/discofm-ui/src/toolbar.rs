//! Header (files, views, audio output) and the audition transport strip.
use super::*;
use discofm_chip::Channel;
use discofm_engine::{ChipProfile, ClockConfig};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

const WHITE_KEYS: [u8; 8] = [0, 2, 4, 5, 7, 9, 11, 12];
/// Black key semitone and the white key it sits after.
const BLACK_KEYS: [(u8, usize); 5] = [(1, 0), (3, 1), (6, 3), (8, 4), (10, 5)];
const KEY_LETTERS: [&str; 13] = [
    "Z", "S", "X", "D", "C", "V", "G", "B", "H", "N", "J", "M", ",",
];

impl Editor {
    pub(super) fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("DiscoFM")
                    .size(21.0)
                    .strong()
                    .color(theme::ACCENT),
            );
            ui.label(RichText::new("YM2612 patch lab").color(theme::FAINT));
            ui.add_space(18.0);
            let fit_label = match (&self.fit.worker, &self.fit.progress) {
                (Some(_), Some(p)) => format!(
                    "Fitting · {:.0}%",
                    100.0 * p.evaluations as f32 / p.budget.max(1) as f32
                ),
                (Some(_), None) => "Fitting · starting".into(),
                _ => "Fitting".into(),
            };
            theme::segmented(
                ui,
                &mut self.fit.view,
                &[
                    (
                        false,
                        "Patch editor",
                        "Edit the instrument and compare it with the reference.",
                    ),
                    (
                        true,
                        &fit_label,
                        "Search for chip settings that reproduce the reference.",
                    ),
                ],
            );
            ui.add_space(12.0);
            let name = self.session_path.as_ref().map_or_else(
                || "Untitled".to_owned(),
                |path| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                },
            );
            let unsaved = self.unsaved();
            ui.label(
                RichText::new(if unsaved { format!("{name} •") } else { name }).color(theme::MUTED),
            )
            .on_hover_text(match (&self.session_path, unsaved) {
                (Some(path), true) => format!("{}\nUnsaved changes", path.display()),
                (Some(path), false) => path.display().to_string(),
                (None, true) => "Not saved yet".into(),
                (None, false) => "New session".into(),
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("🔊 Audio & MIDI", |ui| self.audio_menu(ui))
                    .response
                    .on_hover_text(help::DEVICE);
                ui.separator();
                ui.menu_button("Patch", |ui| self.patch_menu(ui))
                    .response
                    .on_hover_text(help::PATCH_MENU);
                if ui
                    .add_enabled(self.undo.can_redo(), egui::Button::new("Redo"))
                    .on_hover_text("Redo the last undone patch edit (Ctrl+Y).")
                    .clicked()
                {
                    self.history_request = Some(true);
                }
                if ui
                    .add_enabled(self.undo.can_undo(), egui::Button::new("Undo"))
                    .on_hover_text("Undo the last patch edit (Ctrl+Z).")
                    .clicked()
                {
                    self.history_request = Some(false);
                }
                ui.separator();
                if ui
                    .button("Import reference…")
                    .on_hover_text(help::IMPORT)
                    .clicked()
                {
                    self.stop();
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            "Audio and sound banks",
                            &["wav", "sf2", "sf3", "sfz", "dls"],
                        )
                        .add_filter("Sound banks", &["sf2", "sf3", "sfz", "dls"])
                        .add_filter("WAV audio", &["wav"])
                        .pick_file()
                    {
                        self.explicit_reference = true;
                        self.load_reference(path);
                    }
                }
                if ui.button("Save…").on_hover_text(help::SAVE).clicked() {
                    self.save_dialog();
                }
                if ui.button("Open…").on_hover_text(help::OPEN).clicked() {
                    self.stop();
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("DiscoFM session", &["json"])
                        .pick_file()
                    {
                        self.explicit_reference = false;
                        self.load_session(path);
                    }
                }
            });
        });
    }
    fn patch_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("Copy patch")
            .on_hover_text(help::COPY_PATCH)
            .clicked()
        {
            self.copy_patch(ui.ctx());
            ui.close();
        }
        if ui
            .button("Paste patch")
            .on_hover_text(help::PASTE_PATCH)
            .clicked()
        {
            self.paste_from_clipboard();
            ui.close();
        }
        ui.separator();
        if ui
            .button("Import instrument…")
            .on_hover_text(help::IMPORT_PATCH)
            .clicked()
        {
            ui.close();
            self.import_patch_dialog();
        }
        if ui
            .button("Export instrument…")
            .on_hover_text(help::EXPORT)
            .clicked()
        {
            ui.close();
            self.export_dialog();
        }
        ui.separator();
        if ui
            .button("Patch library…")
            .on_hover_text(help::LIBRARY)
            .clicked()
        {
            self.library.open = true;
            self.library.listed = false;
            ui.close();
        }
    }
    /// Returns whether a save was started.
    pub(super) fn save_dialog(&mut self) -> bool {
        self.stop();
        let name = self
            .session_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "instrument.json".into(),
                |n| n.to_string_lossy().into_owned(),
            );
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("DiscoFM session", &["json"])
            .set_file_name(name)
            .save_file()
        {
            let mut session = self.session.clone();
            session.reference_analysis = self.reference.as_ref().map(|_| ReferenceSettings {
                options: self.options.clone(),
                note_on_seconds: self.note_on_marker,
                key_off_seconds: self.key_off_marker,
            });
            let result = self.service.job(Job::Save(path, Box::new(session.clone())));
            if self.report(result).is_some() {
                self.pending_save = Some(Box::new(session));
                return true;
            }
        }
        false
    }
    fn audio_menu(&mut self, ui: &mut egui::Ui) {
        if !self.devices_listed {
            self.devices_listed = true;
            let result = discofm_audio::output_devices();
            if let Some(devices) = self.report(result) {
                self.devices = devices;
            }
        }
        ui.label(theme::caption("Audio output"));
        let previous = self.device_id.clone();
        ui.selectable_value(&mut self.device_id, None, "System default");
        for d in &self.devices {
            ui.selectable_value(&mut self.device_id, Some(d.id.clone()), &d.name);
        }
        if previous != self.device_id {
            self.stop();
            self.playback = None;
            ui.close();
        }
        ui.separator();
        if ui.button("Refresh device list").clicked() {
            self.devices_listed = false;
            self.midi_ports = None;
        }
        ui.separator();
        ui.label(theme::caption("MIDI input"))
            .on_hover_text(help::MIDI);
        if self.midi_ports.is_none() {
            let result = midi::Midi::ports();
            self.midi_ports = Some(self.report(result).unwrap_or_default());
        }
        let current = self.midi.port.clone();
        let mut choice = None;
        if ui.selectable_label(current.is_none(), "None").clicked() {
            choice = Some(None);
        }
        for port in self.midi_ports.iter().flatten() {
            if ui
                .selectable_label(current.as_ref() == Some(port), port)
                .clicked()
            {
                choice = Some(Some(port.clone()));
            }
        }
        if self.midi_ports.as_ref().is_some_and(Vec::is_empty) {
            theme::note(ui, "No MIDI inputs found.");
        }
        if let Some(choice) = choice {
            self.release();
            self.midi_voice = None;
            match choice {
                Some(port) => {
                    let result = self.midi.connect(&port, ui.ctx().clone());
                    if self.report(result).is_some() {
                        self.status = format!("MIDI input: {port}");
                    }
                }
                None => self.midi.disconnect(),
            }
            ui.close();
        }
    }
    pub(super) fn transport(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(
            egui::Layout::left_to_right(egui::Align::Max).with_main_wrap(true),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if ui
                    .add(theme::primary("▶ Play"))
                    .on_hover_text(help::PLAY)
                    .clicked()
                {
                    self.trigger(self.session.audition.note);
                }
                if ui.button("Release").on_hover_text(help::RELEASE).clicked() {
                    self.release();
                }
                if ui.button("⏹ Stop").on_hover_text(help::STOP).clicked() {
                    self.stop();
                }
                ui.separator();
                let audition = &mut self.session.audition;
                theme::field(ui, "Note", help::NOTE, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut audition.note)
                            .range(0..=119)
                            .custom_formatter(|n, _| {
                                format!("{n:.0} · {}", discofm_analysis::note_name(n as i32))
                            })
                            .custom_parser(|s| s.split_whitespace().next()?.parse().ok()),
                    )
                });
                theme::field(ui, "Fine", help::FINE, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut audition.tuning_cents)
                            .range(-100.0..=100.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" ct"),
                    )
                });
                let mut velocity = audition.velocity.get();
                if theme::field(ui, "Velocity", help::VELOCITY, |ui| {
                    ui.add(egui::DragValue::new(&mut velocity).range(0..=127))
                })
                .changed()
                {
                    audition.velocity = discofm_chip::Value::new(velocity).expect("bounded");
                }
                theme::field(ui, "Chip", help::PROFILE, |ui| {
                    egui::ComboBox::from_id_salt("profile")
                        .width(82.0)
                        .selected_text(match audition.profile {
                            ChipProfile::Ym2612Discrete => "YM2612",
                            ChipProfile::Ym3438 => "YM3438",
                            ChipProfile::RawDiagnostic => "Raw",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut audition.profile,
                                ChipProfile::Ym2612Discrete,
                                "YM2612",
                            );
                            ui.selectable_value(
                                &mut audition.profile,
                                ChipProfile::Ym3438,
                                "YM3438",
                            );
                        })
                        .response
                });
                theme::field(ui, "Clock", help::CLOCK, |ui| {
                    egui::ComboBox::from_id_salt("clock")
                        .width(70.0)
                        .selected_text(match audition.clock {
                            ClockConfig::Ntsc => "NTSC",
                            ClockConfig::Pal => "PAL",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut audition.clock, ClockConfig::Ntsc, "NTSC");
                            ui.selectable_value(&mut audition.clock, ClockConfig::Pal, "PAL");
                        })
                        .response
                });
                let mut channel = audition.channel.number();
                theme::field(ui, "Channel", help::CHANNEL, |ui| {
                    ui.add_enabled(
                        self.session.instrument.channel3_special.is_none(),
                        egui::DragValue::new(&mut channel).range(1..=6),
                    )
                });
                audition.channel = Channel::new(channel).expect("bounded");
                let min_gate = discofm_engine::REGISTER_TICKS as f64 / audition.clock.tick_hz();
                theme::field(ui, "Gate", help::GATE, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut audition.key_on_seconds)
                            .range(min_gate..=30.0)
                            .speed(0.01)
                            .suffix(" s"),
                    )
                });
                theme::field(ui, "Length", help::LENGTH, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut audition.total_seconds)
                            .range(f64::MIN_POSITIVE..=30.0)
                            .speed(0.01)
                            .suffix(" s"),
                    )
                });
                ui.separator();
                theme::field(ui, "Octave", help::OCTAVE, |ui| {
                    ui.add(egui::DragValue::new(&mut self.octave).range(-1..=8))
                });
                self.piano(ui);
            },
        );
    }
    /// One octave plus the next C; clicks and the computer keyboard share voices.
    fn piano(&mut self, ui: &mut egui::Ui) {
        let (white_w, height) = (24.0, 42.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(white_w * WHITE_KEYS.len() as f32, height),
            Sense::click_and_drag(),
        );
        let black = |(semitone, after): (u8, usize)| {
            let x = rect.left() + white_w * (after as f32 + 1.0);
            (
                semitone,
                Rect::from_min_max(
                    Pos2::new(x - white_w * 0.32, rect.top()),
                    Pos2::new(x + white_w * 0.32, rect.top() + height * 0.6),
                ),
            )
        };
        let white = |(i, semitone): (usize, &u8)| {
            (
                *semitone,
                Rect::from_min_size(
                    Pos2::new(rect.left() + white_w * i as f32, rect.top()),
                    Vec2::new(white_w, height),
                )
                .shrink2(Vec2::new(1.0, 0.0)),
            )
        };
        let under = |pos: Pos2| {
            BLACK_KEYS
                .map(black)
                .into_iter()
                .chain(WHITE_KEYS.iter().enumerate().map(white))
                .find(|(_, r)| r.contains(pos))
                .map(|(s, _)| s)
        };
        let hovered = response.hover_pos().and_then(under);
        if response.is_pointer_button_down_on()
            && self.pointer_voice.is_none()
            && let Some(semitone) = response.interact_pointer_pos().and_then(under)
            && let Some(note) = self.report(discofm_audio::octave_note(
                self.octave,
                semitone,
                self.session.audition.clock,
            ))
        {
            self.pointer_voice = self.trigger(note);
            self.pointer_semitone = Some(semitone);
        }
        if self.pointer_voice.is_none() {
            self.pointer_semitone = None;
        }
        let held = self
            .held
            .and_then(|(key, _)| piano_keys().iter().position(|k| *k == key))
            .map(|i| i as u8);
        let active = |s: u8| held == Some(s) || self.pointer_semitone == Some(s);
        let painter = ui.painter_at(rect.expand(1.0));
        for (semitone, r) in WHITE_KEYS.iter().enumerate().map(white) {
            let fill = if active(semitone) {
                theme::ACCENT
            } else if hovered == Some(semitone) {
                Color32::from_gray(238)
            } else {
                Color32::from_gray(214)
            };
            painter.rect_filled(
                r,
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 3,
                    se: 3,
                },
                fill,
            );
            painter.text(
                r.center_bottom() - Vec2::new(0.0, 4.0),
                egui::Align2::CENTER_BOTTOM,
                KEY_LETTERS[usize::from(semitone)],
                egui::FontId::monospace(9.0),
                Color32::from_gray(110),
            );
        }
        for (semitone, r) in BLACK_KEYS.map(black) {
            let fill = if active(semitone) {
                theme::ACCENT
            } else if hovered == Some(semitone) {
                Color32::from_gray(70)
            } else {
                Color32::from_gray(28)
            };
            painter.rect(
                r,
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 2,
                    se: 2,
                },
                fill,
                Stroke::new(1.0, Color32::from_gray(10)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                r.center_bottom() - Vec2::new(0.0, 3.0),
                egui::Align2::CENTER_BOTTOM,
                KEY_LETTERS[usize::from(semitone)],
                egui::FontId::monospace(8.0),
                Color32::from_gray(150),
            );
        }
        if let Some(semitone) = hovered {
            let name =
                discofm_audio::octave_note(self.octave, semitone, self.session.audition.clock)
                    .map_or_else(
                        |_| "out of range".to_owned(),
                        |n| discofm_analysis::note_name(i32::from(n)),
                    );
            response.on_hover_text(format!(
                "{name} · computer key {}",
                KEY_LETTERS[usize::from(semitone)]
            ));
        }
    }
}
