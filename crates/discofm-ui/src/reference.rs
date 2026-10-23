//! Reference side panel: source, A/B listening, analysis setup and read-outs.
use super::*;
use crate::plots::{Chart, Series, Unit};
use discofm_analysis::ChannelMode;

impl Editor {
    fn clips_ready(&self) -> bool {
        !self.reference_loading
            && !self.session_loading
            && self
                .preview
                .as_ref()
                .is_some_and(|p| p.revision == self.revision)
    }
    pub(super) fn reference_panel(&mut self, ui: &mut egui::Ui) {
        theme::title(ui, "Reference", "A = recording · B = chip");
        if self.reference.is_none() && self.bank.is_none() && !self.reference_loading {
            self.empty_reference(ui);
            return;
        }
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.bank_panel(ui);
            if let Some(path) = &self.session.reference_path {
                let name = std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                ui.add(
                    egui::Label::new(RichText::new(name).strong().color(theme::REFERENCE))
                        .truncate(),
                )
                .on_hover_text(path);
            }
            if self.reference_loading {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    theme::note(ui, "Loading…");
                });
            } else if let Some(a) = &self.reference {
                theme::note(
                    ui,
                    &format!(
                        "{} Hz · {:.3} s · decoded as stereo",
                        a.sample_rate,
                        a.frames.len() as f64 / f64::from(a.sample_rate)
                    ),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = self.clips_ready();
                let has_a = self
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.comparison.is_some());
                if ui
                    .add_enabled(
                        ready && has_a,
                        egui::Button::new(RichText::new("▶ A").color(theme::REFERENCE)),
                    )
                    .on_hover_text(help::PLAY_A)
                    .clicked()
                {
                    self.play_clip(true);
                }
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new(RichText::new("▶ B").color(theme::ACCENT)),
                    )
                    .on_hover_text(help::PLAY_B)
                    .clicked()
                {
                    self.play_clip(false);
                }
                if ui.button("⏹").on_hover_text(help::STOP).clicked() {
                    self.stop();
                }
                ui.checkbox(&mut self.match_ab_level, "Level-match")
                    .on_hover_text(help::MATCH_LEVEL);
            });
            theme::note(
                ui,
                "A and B use the same region, channel and preparation; each clip restarts.",
            );
        });
        ui.add_space(8.0);
        self.analysis_setup(ui);
        ui.add_space(8.0);
        self.analysis_results(ui);
    }
    fn empty_reference(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(16, 28))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🎵").size(30.0).color(theme::FAINT));
                    ui.label(RichText::new("No reference loaded").strong());
                    ui.add_space(2.0);
                    theme::note(
                        ui,
                        "Drop a WAV, SF2, SF3, SFZ or DLS file anywhere on the window, or import one.",
                    );
                    ui.add_space(8.0);
                    if ui
                        .add(theme::primary("Import reference…"))
                        .on_hover_text(help::IMPORT)
                        .clicked()
                    {
                        self.stop();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio and sound banks", &["wav", "sf2", "sf3", "sfz", "dls"])
                            .pick_file()
                        {
                            self.explicit_reference = true;
                            self.load_reference(path);
                        }
                    }
                    ui.add_space(6.0);
                    theme::note(
                        ui,
                        "Without a reference you can still edit and play the patch.",
                    );
                });
            });
    }
    fn analysis_setup(&mut self, ui: &mut egui::Ui) {
        let duration = self
            .reference
            .as_ref()
            .map_or(1.0, |a| a.frames.len() as f64 / f64::from(a.sample_rate));
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::title(ui, "Analysis", "");
            ui.label(theme::caption("Region")).on_hover_text(help::REGION);
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.options.start_seconds)
                        .range(0.0..=duration)
                        .speed(0.005)
                        .prefix("from ")
                        .suffix(" s"),
                )
                .on_hover_text(help::REGION);
                let mut end = self.options.end_seconds.unwrap_or(duration);
                if ui
                    .add(
                        egui::DragValue::new(&mut end)
                            .range(0.0..=duration)
                            .speed(0.005)
                            .prefix("to ")
                            .suffix(" s"),
                    )
                    .on_hover_text(help::REGION)
                    .changed()
                {
                    self.options.end_seconds = Some(end);
                }
                if ui
                    .small_button(if duration <= 30.0 { "All" } else { "First 30 s" })
                    .on_hover_text("Reset the region to the whole file (or its first 30 s).")
                    .clicked()
                {
                    self.options.start_seconds = 0.0;
                    self.options.end_seconds = (duration > 30.0).then_some(30.0);
                }
            });
            ui.add_space(4.0);
            ui.label(theme::caption("Note markers"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.note_on_marker)
                        .range(0.0..=duration)
                        .speed(0.005)
                        .prefix("on ")
                        .suffix(" s"),
                )
                .on_hover_text(help::NOTE_ON);
                ui.add(
                    egui::DragValue::new(&mut self.key_off_marker)
                        .range(0.0..=duration)
                        .speed(0.005)
                        .prefix("off ")
                        .suffix(" s"),
                )
                .on_hover_text(help::KEY_OFF);
            });
            ui.add_space(4.0);
            ui.label(theme::caption("Pitch"));
            ui.horizontal(|ui| {
                let mut manual = self.options.pitch_override_hz.is_some();
                if ui
                    .checkbox(&mut manual, "Manual")
                    .on_hover_text(help::MANUAL_PITCH)
                    .changed()
                {
                    self.options.pitch_override_hz = manual.then_some(self.manual_hz);
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut self.manual_hz)
                            .range(20.0..=4000.0)
                            .speed(0.1)
                            .max_decimals(2)
                            .suffix(" Hz"),
                    )
                    .on_hover_text(help::MANUAL_PITCH)
                    .changed()
                    && manual
                {
                    self.options.pitch_override_hz = Some(self.manual_hz);
                }
                ui.add(
                    egui::DragValue::new(&mut self.manual_note)
                        .range(16..=107)
                        .custom_formatter(|n, _| discofm_analysis::note_name(n as i32))
                        .custom_parser(|s| {
                            (16..=107).find(|n| discofm_analysis::note_name(*n).eq_ignore_ascii_case(s.trim())).map(f64::from).or_else(|| s.trim().parse().ok())
                        }),
                )
                .on_hover_text(help::MANUAL_NOTE);
                if ui
                    .small_button("Use note")
                    .on_hover_text(help::MANUAL_NOTE)
                    .clicked()
                {
                    self.manual_hz = discofm_engine::midi_hz(self.manual_note)
                        .expect("bounded MIDI")
                        .clamp(20.0, 4000.0);
                    self.options.pitch_override_hz = Some(self.manual_hz);
                }
            });
            ui.add_space(4.0);
            egui::CollapsingHeader::new(theme::caption("Preparation"))
                .id_salt("preparation")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::segmented(
                            ui,
                            &mut self.options.channel,
                            &[
                                (ChannelMode::Mix, "Mix", help::CHANNEL_MODE),
                                (ChannelMode::Left, "Left", help::CHANNEL_MODE),
                                (ChannelMode::Right, "Right", help::CHANNEL_MODE),
                            ],
                        );
                        egui::ComboBox::from_id_salt("fft")
                            .width(92.0)
                            .selected_text(format!("FFT {}", self.options.fft_size))
                            .show_ui(ui, |ui| {
                                for size in [512, 1024, 2048, 4096, 8192] {
                                    ui.selectable_value(
                                        &mut self.options.fft_size,
                                        size,
                                        size.to_string(),
                                    );
                                }
                            })
                            .response
                            .on_hover_text(help::FFT);
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.options.remove_dc, "Remove DC")
                            .on_hover_text(help::REMOVE_DC);
                        ui.checkbox(&mut self.options.normalize, "Normalize")
                            .on_hover_text(help::NORMALIZE);
                        ui.checkbox(&mut self.options.trim_silence, "Trim silence")
                            .on_hover_text(help::TRIM);
                    });
                });
            ui.add_space(6.0);
            let busy = self.pending_reference_apply.is_some();
            let button = if busy {
                egui::Button::new("Analysing…")
            } else {
                theme::primary("Analyse & apply to audition note")
            };
            if ui
                .add_enabled(
                    self.reference.is_some()
                        && !self.reference_loading
                        && !self.session_loading
                        && !busy,
                    button.min_size(egui::vec2(ui.available_width(), 28.0)),
                )
                .on_hover_text(help::APPLY_ANALYSIS)
                .clicked()
            {
                self.request_reference_apply = true;
            }
            theme::note(
                ui,
                "Sets pitch, gate and length from the markers. Key-off is never inferred from the decay.",
            );
        });
    }
    fn analysis_results(&mut self, ui: &mut egui::Ui) {
        let duration = self
            .reference
            .as_ref()
            .map_or(1.0, |a| a.frames.len() as f64 / f64::from(a.sample_rate));
        let Some(preview) = self
            .preview
            .as_ref()
            .filter(|p| p.revision == self.revision && !self.reference_loading)
        else {
            return;
        };
        let (Some(a), Some(curves)) = (&preview.reference, &preview.curves.reference) else {
            return;
        };
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::title(ui, "Detected", "monophonic estimate");
            if let Some(pitch) = &a.pitch {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(pitch.note_name())
                            .size(26.0)
                            .strong()
                            .color(theme::REFERENCE),
                    );
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("{:.2} Hz", pitch.hz))
                                .monospace()
                                .color(theme::TEXT),
                        );
                        ui.label(
                            RichText::new(format!("{:+.1} cents", pitch.cents()))
                                .small()
                                .color(theme::MUTED),
                        );
                    });
                });
                let confident = pitch.confidence >= 0.5;
                ui.add(
                    egui::ProgressBar::new(pitch.confidence as f32)
                        .desired_height(6.0)
                        .fill(if confident { theme::GOOD } else { theme::WARN }),
                )
                .on_hover_text(format!(
                    "YIN confidence {:.1}%. Fitting needs at least 50% unless a manual pitch or percussion mode is used.",
                    pitch.confidence * 100.0
                ));
            } else {
                ui.label(RichText::new("No stable pitch").strong().color(theme::WARN));
                theme::note(ui, "Set a manual pitch, or use percussion mode when fitting.");
            }
            ui.add_space(4.0);
            theme::stats(
                ui,
                "reference-stats",
                &[
                    (
                        "Used pitch",
                        a.used_pitch_hz.map_or("—".into(), |hz| format!("{hz:.2} Hz")),
                        "Pitch passed on to the comparison and fitting (manual pitch wins).",
                    ),
                    (
                        "Peak / RMS",
                        format!("{:.4} / {:.4}", a.peak, a.rms),
                        "",
                    ),
                    (
                        "DC L / R",
                        format!("{:+.5} / {:+.5}", a.dc[0], a.dc[1]),
                        "Mean offset of each channel in the region.",
                    ),
                    (
                        "Gain",
                        format!("×{:.3}", a.normalization_gain),
                        "Normalisation gain applied to the analysed copy.",
                    ),
                    (
                        "Clipped",
                        a.clipped_samples.to_string(),
                        "Samples with |value| ≥ 1.",
                    ),
                    (
                        "Region",
                        format!("{:.3} – {:.3} s", a.region_start_seconds, a.region_end_seconds),
                        "Effective region after trimming.",
                    ),
                ],
            );
            ui.add_space(6.0);
            ui.label(theme::caption("Envelope"));
            let top = plots::max_y(&[&curves.peak], 0.001);
            Chart::new(96.0, [0.0, duration], [0.0, top])
                .units(Unit::Seconds, Unit::Level)
                .show(
                    ui,
                    &[
                        Series::line("peak", &curves.peak, egui::Color32::from_gray(100)),
                        Series::area("rms", &curves.rms, theme::REFERENCE),
                    ],
                );
            if !a.transients.is_empty() {
                theme::note(
                    ui,
                    &format!(
                        "Transients: {}",
                        a.transients
                            .iter()
                            .take(10)
                            .map(|t| format!("{t:.3}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            ui.add_space(6.0);
            ui.label(theme::caption("Harmonics"))
                .on_hover_text("Local FFT peaks near multiples of the used pitch.");
            egui::Grid::new("harmonics")
                .striped(true)
                .num_columns(3)
                .spacing([18.0, 2.0])
                .show(ui, |ui| {
                    for h in ["#", "Hz", "dBFS"] {
                        ui.label(RichText::new(h).small().color(theme::FAINT));
                    }
                    ui.end_row();
                    for h in a.harmonics.iter().take(8) {
                        ui.monospace(h.number.to_string());
                        ui.monospace(format!("{:.1}", h.hz));
                        ui.monospace(format!("{:.1}", 20.0 * h.magnitude.max(1e-6).log10()));
                        ui.end_row();
                    }
                });
        });
    }
}
