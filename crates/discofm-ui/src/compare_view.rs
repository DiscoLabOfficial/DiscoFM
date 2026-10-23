//! A/B comparison card below the patch editor.
use super::*;
use crate::plots::{Chart, Series, Unit};

pub(crate) fn banner(ui: &mut egui::Ui, color: egui::Color32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(add);
        });
}

impl Editor {
    pub(super) fn comparison(&mut self, ui: &mut egui::Ui) {
        if let Some(run) = self
            .session
            .fitting
            .as_ref()
            .and_then(|s| s.runs.get(self.fit.run))
            && let Some(candidate) = run.candidates.get(self.fit.candidate)
            && (candidate.instrument != self.session.instrument
                || run.context.audition != self.session.audition)
        {
            let mut apply = false;
            banner(ui, theme::WARN, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Fitting candidate #{} is selected but not applied — B is still the editor patch.",
                        self.fit.candidate + 1
                    ))
                    .color(theme::WARN),
                );
                apply = ui
                    .button("Apply candidate")
                    .on_hover_text(help::APPLY)
                    .clicked();
            });
            if apply {
                self.apply_candidate(false);
            }
            ui.add_space(8.0);
        }
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Compare").size(15.0).strong());
                ui.add_space(8.0);
                theme::segmented(
                    ui,
                    &mut self.tab,
                    &[
                        (
                            0,
                            "Waveform",
                            "Min/max waveform of A and B on a shared scale.",
                        ),
                        (1, "Spectrum", "Average magnitude spectrum, log frequency."),
                        (2, "Spectrogram", "Short-time spectra over time."),
                        (
                            3,
                            "Dynamics",
                            "RMS envelope, spectral centroid and spectral flux over time.",
                        ),
                    ],
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::legend(ui, theme::ACCENT, "B · chip");
                    if self.reference.is_some() {
                        theme::legend(ui, theme::REFERENCE, "A · reference");
                    }
                });
            });
            ui.add_space(4.0);
            self.comparison_body(ui);
        });
    }
    fn comparison_body(&mut self, ui: &mut egui::Ui) {
        let Some(current) = self
            .preview
            .as_ref()
            .filter(|p| p.revision == self.revision)
        else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                theme::note(ui, "Rendering the chip preview…");
            });
            return;
        };
        let comparison = current.comparison.as_ref();
        let (a, b) = (
            current.curves.prepared_reference.as_ref(),
            &current.curves.generated,
        );
        let seconds = comparison.map_or(current.generated.original_seconds, |c| {
            c.end_seconds - c.start_seconds
        });
        if let Some(c) = comparison {
            theme::note(
                ui,
                &format!(
                    "Region {:.3}–{:.3} s · B gain ×{:.3} ({:+.1} dB) · shared playback attenuation ×{:.3}",
                    c.start_seconds,
                    c.end_seconds,
                    c.generated_gain,
                    20.0 * c.generated_gain.log10(),
                    c.playback_gain
                ),
            );
        }
        let expected = comparison.map_or(seconds, |c| c.end_seconds - self.note_on_marker);
        if (current.audio.frames.len() as f64 / f64::from(current.audio.sample_rate) - expected)
            .abs()
            > 0.002
        {
            banner(ui, theme::WARN, |ui| {
                ui.label(
                    RichText::new("The chip render length differs from the reference region. Use “Analyse & apply”, or apply a fitting candidate.")
                        .color(theme::WARN),
                );
            });
        }
        ui.add_space(4.0);
        match self.tab {
            0 => {
                let peak = self
                    .reference_wave
                    .iter()
                    .chain(&self.generated_wave)
                    .map(|p| p[1].abs())
                    .fold(1.0, f64::max);
                if comparison.is_some() {
                    ui.label(theme::caption("A · prepared reference"));
                    Chart::new(104.0, [0.0, seconds], [-peak, peak]).show(
                        ui,
                        &[Series::min_max("A", &self.reference_wave, theme::REFERENCE)],
                    );
                }
                ui.label(theme::caption("B · chip (comparison gain)"));
                Chart::new(104.0, [0.0, seconds], [-peak, peak]).show(
                    ui,
                    &[Series::min_max("B", &self.generated_wave, theme::ACCENT)],
                );
                theme::note(
                    ui,
                    &format!(
                        "Shared scale ±{peak:.2} · same interval and origin · one gain, never per window"
                    ),
                );
            }
            1 => plots::spectrum(ui, a, Some(b)),
            2 => {
                let reference = comparison.map(|c| &c.analysis);
                if let Some(r) = reference {
                    ui.label(theme::caption("A · prepared reference"));
                    plots::spectrogram(
                        ui,
                        self.reference_texture.as_ref(),
                        Some(&r.spectrogram),
                        r.region_end_seconds - r.region_start_seconds,
                        r.sample_rate,
                    );
                    ui.add_space(4.0);
                }
                ui.label(theme::caption("B · chip"));
                plots::spectrogram(
                    ui,
                    self.generated_texture.as_ref(),
                    Some(&current.generated.spectrogram),
                    current.generated.original_seconds,
                    current.generated.sample_rate,
                );
                theme::note(
                    ui,
                    "Log frequency from 20 Hz · colour −100…0 dBFS · hover for values",
                );
            }
            _ => {
                let xmax = comparison
                    .map_or(0.0, |c| c.analysis.original_seconds)
                    .max(current.generated.original_seconds)
                    .max(1e-3);
                let empty: &[[f64; 2]] = &[];
                let pick = |f: fn(&plots::Curves) -> &Vec<[f64; 2]>| {
                    (a.map_or(empty, |c| f(c).as_slice()), f(b).as_slice())
                };
                for (title, help, field, height, unit, floor) in [
                    (
                        "RMS envelope",
                        "Short-time RMS level of A and B on one scale.",
                        (|c| &c.rms) as fn(&plots::Curves) -> &Vec<[f64; 2]>,
                        90.0,
                        Unit::Level,
                        0.001,
                    ),
                    (
                        "Spectral centroid",
                        "Brightness: the spectrum's centre of mass over time.",
                        |c| &c.centroid,
                        90.0,
                        Unit::Hz,
                        100.0,
                    ),
                    (
                        "Spectral flux",
                        "Positive frame-to-frame spectral change; peaks mark onsets and movement.",
                        |c| &c.flux,
                        90.0,
                        Unit::Level,
                        1.0,
                    ),
                ] {
                    let (sa, sb) = pick(field);
                    ui.label(theme::caption(title)).on_hover_text(help);
                    Chart::new(height, [0.0, xmax], [0.0, plots::max_y(&[sa, sb], floor)])
                        .units(Unit::Seconds, unit)
                        .show(
                            ui,
                            &[
                                Series::line("A", sa, theme::REFERENCE),
                                Series::line("B", sb, theme::ACCENT),
                            ],
                        );
                    ui.add_space(2.0);
                }
            }
        }
    }
}
