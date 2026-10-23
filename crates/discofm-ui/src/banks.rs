use super::*;
impl Editor {
    pub(super) fn load_bank_sample(&mut self, bank: Arc<SoundBank>, selection: SampleSelection) {
        self.stop();
        self.pending_reference_apply = None;
        let id = self.reference_id.wrapping_add(1);
        if self
            .report(self.service.job(Job::Sample(id, bank, selection)))
            .is_some()
        {
            self.fit.cancel(false);
            self.reference_id = id;
            self.reference_loading = true;
            self.status = "Decoding bank sample…".into();
        }
    }
    fn hear_sample(&mut self) {
        if self.reference_loading || self.session_loading {
            return;
        }
        if let Some(audio) = self.reference.clone()
            && self.ensure_playback()
            && let Some(playback) = self.playback.as_mut()
        {
            let result = playback.controls().play_clip(audio);
            if self.report(result).is_some() {
                self.listening = Listening::Reference;
            }
        }
    }
    pub(super) fn bank_panel(&mut self, ui: &mut egui::Ui) {
        let Some(bank) = self.bank.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            theme::badge(ui, &bank.format.to_string(), theme::REFERENCE);
            ui.add(
                egui::Label::new(
                    RichText::new(bank.path.file_name().unwrap_or_default().to_string_lossy())
                        .strong(),
                )
                .truncate(),
            )
            .on_hover_text(bank.path.display().to_string());
        });
        theme::note(
            ui,
            &format!(
                "{} samples · raw one-shot playback, no MIDI preset articulation",
                bank.samples.len()
            ),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.bank_filter)
                .hint_text("🔍 Filter samples…")
                .desired_width(f32::INFINITY),
        );
        let filter = self.bank_filter.to_lowercase();
        let visible: Vec<_> = bank
            .samples
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
            .collect();
        let selected = self
            .session
            .reference_sample
            .as_ref()
            .filter(|_| {
                self.session.reference_path.as_deref() == Some(bank.path.to_string_lossy().as_ref())
            })
            .map(|s| s.index);
        let mut choose = None;
        egui::Frame::new()
            .fill(theme::INSET)
            .corner_radius(6)
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("bank_samples")
                    .max_height(170.0)
                    .auto_shrink([false, true])
                    .show_rows(ui, 20.0, visible.len(), |ui, rows| {
                        for row in rows {
                            let sample = visible[row];
                            let text =
                                RichText::new(format!("{:>3}  {}", sample.index + 1, sample.name));
                            let text = if sample.warning.is_some() {
                                text.color(theme::FAINT)
                            } else {
                                text
                            };
                            let response = ui.add(
                                egui::Button::selectable(selected == Some(sample.index), text)
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            if response.clicked() {
                                choose = Some(sample.index);
                            }
                            if let Some(warning) = &sample.warning {
                                response.on_hover_text(warning);
                            }
                        }
                    });
                if visible.is_empty() {
                    theme::note(ui, "No sample matches the filter.");
                }
            });
        if let Some(index) = choose {
            self.explicit_reference = true;
            self.pending_reference_settings = None;
            if let Some(selection) = self.report(bank.selection(index)) {
                self.load_bank_sample(bank.clone(), selection);
            }
        }
        if let Some(sample) = selected.and_then(|i| bank.samples.get(i)) {
            let mut info = format!(
                "{} channel(s) · root {} · tuning {} cents",
                sample.channels,
                sample
                    .root_key
                    .map(|n| discofm_analysis::note_name(i32::from(n)))
                    .unwrap_or_else(|| "not set".into()),
                sample.pitch_correction
            );
            if let Some([start, end]) = sample.loop_frames {
                info.push_str(&format!(" · loop {start}–{end} (informational)"));
            }
            theme::note(ui, &info);
            if let Some(warning) = &sample.warning {
                ui.label(RichText::new(warning).small().color(theme::WARN));
            }
        }
        if ui
            .add_enabled(
                selected.is_some()
                    && !self.reference_loading
                    && !self.session_loading
                    && self.reference.is_some(),
                egui::Button::new("▶ Play whole sample"),
            )
            .on_hover_text("Play the complete decoded sample, ignoring the analysis region.")
            .clicked()
        {
            self.hear_sample();
        }
        ui.separator();
    }
}
