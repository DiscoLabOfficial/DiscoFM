use crate::{
    Editor, help,
    plots::{Chart, Series, Unit},
    theme,
};
use discofm_fit::{
    ComputeMode, DEFAULT_INHARMONIC_WEIGHT, FitError, FitInput, FitMode, FitProgress, FitStatus,
    FitWorker, FittingState, LossWeights, ModePolicy, OperatorField, Parameter, all_parameters,
};
use eframe::egui::{self, RichText};
use std::time::Instant;

const PRESETS: [(&str, usize, usize, &str); 3] = [
    ("Quick", 256, 16, "256 evaluations, population 16."),
    ("Balanced", 512, 16, "512 evaluations, population 16."),
    (
        "Deep",
        2048,
        32,
        "2048 evaluations, population 32. Slow, usually better.",
    ),
];

#[derive(Default)]
pub(super) struct FitUi {
    pub worker: Option<FitWorker>,
    pub progress: Option<FitProgress>,
    pub view: bool,
    pub run: usize,
    pub candidate: usize,
    pub message: String,
    pub discard: bool,
    pub play_revision: Option<usize>,
    started: Option<Instant>,
}
impl FitUi {
    pub fn cancel(&mut self, discard: bool) {
        if let Some(worker) = &self.worker {
            worker.cancel();
            self.discard |= discard;
            self.message = "Cancelling; finished candidates are kept".into();
        }
        self.play_revision = None;
    }
}
impl Editor {
    pub(super) fn start_fit(&mut self, refine: bool) {
        if self.fit.worker.is_some() {
            return;
        }
        let Some(reference) = self.reference.clone() else {
            self.error = Some("Load a reference before fitting".into());
            return;
        };
        let state = self
            .session
            .fitting
            .get_or_insert_with(FittingState::default);
        let selected = state
            .runs
            .get(self.fit.run)
            .and_then(|r| r.candidates.get(self.fit.candidate));
        let initial = if refine {
            let Some(candidate) = selected else {
                return;
            };
            candidate.instrument.clone()
        } else {
            self.session.instrument.clone()
        };
        let seeds = if refine {
            state
                .runs
                .get(self.fit.run)
                .map(|run| {
                    run.candidates
                        .iter()
                        .map(|c| c.instrument.clone())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut audition = self.session.audition.clone();
        if let Some(channel) = initial.required_channel() {
            audition.channel = channel;
        }
        let input = FitInput {
            reference,
            options: self.options.clone(),
            note_on_seconds: self.note_on_marker,
            key_off_seconds: self.key_off_marker,
            initial,
            audition,
            config: state.config.clone(),
            locks: state.locks.clone(),
            seeds,
            refine,
        };
        let c = &input.config;
        discofm_log::info(
            "fitting",
            format!(
                "{} started: {:?}, {} evaluations, population {}, workers {}, compute {:?}, seed {}, {} seeds, reference {} frames at {} Hz, render {:.3} s",
                if refine { "refine" } else { "search" },
                c.mode,
                c.evaluations,
                c.population,
                c.workers,
                c.compute,
                c.seed,
                input.seeds.len(),
                input.reference.frames.len(),
                input.reference.sample_rate,
                input.key_off_seconds - input.note_on_seconds,
            ),
        );
        if let Some(worker) = self.report(FitWorker::start(input)) {
            self.fit.worker = Some(worker);
            self.fit.progress = None;
            self.fit.started = Some(Instant::now());
            self.fit.discard = false;
            self.fit.view = true;
            self.fit.message = "Searching real chip settings…".into();
        }
    }
    pub(super) fn poll_fit(&mut self) {
        let Some(worker) = &self.fit.worker else {
            return;
        };
        let snapshot = worker.poll();
        if let Some(progress) = snapshot.progress.filter(|_| !self.fit.discard) {
            self.fit.progress = Some(progress);
        }
        if let Some(result) = snapshot.result {
            self.fit.worker = None;
            self.fit.started = None;
            if self.fit.discard {
                self.fit.message = "Previous run discarded when the session changed".into();
                return;
            }
            match result {
                Ok(outcome) => {
                    let best = outcome
                        .candidates
                        .first()
                        .map_or(outcome.initial_loss.total, |c| c.loss.total);
                    discofm_log::info(
                        "fitting",
                        format!(
                            "{:?}: {} evaluations, {} renders, {:.1} s, loss {:.6} -> {best:.6}, {} ({}){}",
                            outcome.status,
                            outcome.evaluations,
                            outcome.renders,
                            outcome.elapsed_seconds,
                            outcome.initial_loss.total,
                            outcome.performance.backend,
                            outcome.performance.adapter.as_deref().unwrap_or("no GPU"),
                            if outcome.performance.note.is_empty() {
                                String::new()
                            } else {
                                format!(", {}", outcome.performance.note)
                            }
                        ),
                    );
                    self.fit.message = format!(
                        "{} · {} evaluations / {} renders · {:.1} s · loss {:.5} → {:.5}",
                        if outcome.status == FitStatus::Completed {
                            "Finished"
                        } else {
                            "Cancelled"
                        },
                        outcome.evaluations,
                        outcome.renders,
                        outcome.elapsed_seconds,
                        outcome.initial_loss.total,
                        best
                    );
                    let state = self
                        .session
                        .fitting
                        .get_or_insert_with(FittingState::default);
                    state.push(outcome);
                    self.fit.run = state.runs.len() - 1;
                    self.fit.candidate = 0;
                }
                Err(FitError::Cancelled) => {
                    discofm_log::info("fitting", "cancelled before the first candidate");
                    self.fit.message = "Cancelled before the first candidate".into()
                }
                Err(error) => {
                    discofm_log::error("fitting", &error);
                    self.fit.message = "Fitting stopped by an error".into();
                    self.error = Some(error.to_string());
                }
            }
        }
    }
    pub(super) fn apply_candidate(&mut self, play: bool) {
        let Some(run) = self
            .session
            .fitting
            .as_ref()
            .and_then(|s| s.runs.get(self.fit.run))
        else {
            return;
        };
        let Some(candidate) = run.candidates.get(self.fit.candidate) else {
            return;
        };
        let patch = candidate.instrument.clone();
        let audition = run.context.audition.clone();
        self.match_ab_level = run.config.match_level;
        self.stop();
        self.playback = None;
        self.session.instrument = patch;
        self.session.audition = audition;
        self.dirty();
        if play {
            self.fit.play_revision = Some(self.revision);
        }
        self.status = "Candidate applied to the editor".into();
    }
    pub(super) fn fitting_panel(&mut self, ui: &mut egui::Ui) {
        let running = self.fit.worker.is_some();
        let ready = self.reference.is_some() && !self.reference_loading && !self.session_loading;
        ui.columns(2, |columns| {
            self.fit_setup(&mut columns[0], running);
            self.fit_run(&mut columns[1], running, ready);
        });
        ui.add_space(10.0);
        self.fit_results(ui, running, ready);
    }
    fn fit_setup(&mut self, ui: &mut egui::Ui, running: bool) {
        let editor_special = self.session.instrument.channel3_special.is_some();
        let state = self
            .session
            .fitting
            .get_or_insert_with(FittingState::default);
        let special = editor_special
            || state
                .runs
                .get(self.fit.run)
                .and_then(|run| run.candidates.get(self.fit.candidate))
                .is_some_and(|candidate| candidate.instrument.channel3_special.is_some());
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(!running, |ui| {
                theme::title(ui, "Search setup", "inverse synthesis");
                ui.horizontal(|ui| {
                    ui.label(theme::caption("Mode"));
                    theme::segmented(
                        ui,
                        &mut state.config.mode,
                        &[
                            (FitMode::Basic, "Basic", help::FIT_MODE_BASIC),
                            (FitMode::Advanced, "Advanced", help::FIT_MODE_ADVANCED),
                        ],
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(theme::caption("Budget"));
                    let mut preset = PRESETS
                        .iter()
                        .position(|(_, e, p, _)| {
                            *e == state.config.evaluations && *p == state.config.population
                        })
                        .unwrap_or(usize::MAX);
                    let options = PRESETS
                        .iter()
                        .enumerate()
                        .map(|(i, (label, _, _, help))| (i, *label, *help))
                        .collect::<Vec<_>>();
                    if theme::segmented(ui, &mut preset, &options) {
                        state.config.evaluations = PRESETS[preset].1;
                        state.config.population = PRESETS[preset].2;
                    }
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    theme::field(ui, "Evaluations", help::BUDGET, |ui| {
                        ui.add(
                            egui::DragValue::new(&mut state.config.evaluations)
                                .range(24..=20000)
                                .speed(8),
                        )
                    });
                    theme::field(ui, "Population", help::POPULATION, |ui| {
                        ui.add(egui::DragValue::new(&mut state.config.population).range(4..=64))
                    });
                    theme::field(ui, "Seed", help::SEED, |ui| {
                        ui.add(egui::DragValue::new(&mut state.config.seed))
                    });
                    theme::field(ui, "Max length", help::MAX_SECONDS, |ui| {
                        ui.add(
                            egui::DragValue::new(&mut state.config.max_seconds)
                                .range(0.05..=30.0)
                                .suffix(" s")
                                .speed(0.1),
                        )
                    });
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(theme::caption("Spectral analysis"));
                    theme::segmented(
                        ui,
                        &mut state.config.compute,
                        &[
                            (ComputeMode::Cpu, "CPU", help::COMPUTE_CPU),
                            (ComputeMode::Auto, "Auto (experimental)", help::COMPUTE_AUTO),
                            (ComputeMode::Gpu, "GPU (experimental)", help::COMPUTE_GPU),
                        ],
                    );
                });
                ui.checkbox(&mut state.config.match_level, "Compensate recording gain")
                    .on_hover_text(help::RECORDING_GAIN);
                if ui
                    .checkbox(&mut state.config.unpitched, "Percussion / unpitched")
                    .on_hover_text(help::UNPITCHED)
                    .changed()
                {
                    state.config.weights = if state.config.unpitched {
                        LossWeights::unpitched()
                    } else {
                        LossWeights::default()
                    };
                }
                ui.add_space(4.0);
                egui::CollapsingHeader::new(theme::caption("Search tuning & loss weights"))
                    .id_salt("fit-advanced")
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            theme::field(ui, "Workers", help::WORKERS, |ui| {
                                ui.add(egui::DragValue::new(&mut state.config.workers).range(0..=16))
                            });
                            theme::field(ui, "Candidates", help::CANDIDATES, |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut state.config.candidates).range(2..=16),
                                )
                            });
                            ui.checkbox(&mut state.config.adaptive, "Self-adapting F/CR")
                                .on_hover_text(help::DE_ADAPTIVE);
                            let mut screening = state.config.screening > 0;
                            if ui
                                .checkbox(&mut screening, "ymfm screening")
                                .on_hover_text(help::SCREENING)
                                .changed()
                            {
                                state.config.screening = usize::from(screening);
                            }
                            theme::field(ui, "DE · F", help::DE_F, |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut state.config.differential_weight)
                                        .range(0.1..=2.0)
                                        .speed(0.01),
                                )
                            });
                            theme::field(ui, "DE · CR", help::DE_CR, |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut state.config.crossover)
                                        .range(0.0..=1.0)
                                        .speed(0.01),
                                )
                            });
                        });
                        ui.add_space(4.0);
                        ui.add_enabled_ui(state.config.mode == FitMode::Advanced, |ui| {
                            egui::Grid::new("policies")
                                .num_columns(2)
                                .spacing([10.0, 4.0])
                                .show(ui, |ui| {
                                    for (label, policy) in [
                                        ("LFO / vibrato", &mut state.config.lfo_policy),
                                        ("AM / tremolo", &mut state.config.am_policy),
                                        ("Channel 3 special", &mut state.config.channel3_policy),
                                    ] {
                                        ui.label(label).on_hover_text(help::POLICY);
                                        ui.push_id(label, |ui| {
                                            theme::segmented(
                                                ui,
                                                policy,
                                                &[
                                                    (ModePolicy::Auto, "Auto", help::POLICY),
                                                    (ModePolicy::Preserve, "Keep", help::POLICY),
                                                ],
                                            );
                                        });
                                        ui.end_row();
                                    }
                                });
                        });
                        ui.add_enabled(
                            special
                                && state.config.mode == FitMode::Advanced
                                && state.config.channel3_policy == ModePolicy::Preserve,
                            egui::Checkbox::new(
                                &mut state.config.fit_special_frequencies,
                                "Search S1–S3 special FNUM/BLOCK",
                            ),
                        )
                        .on_hover_text(help::SPECIAL_FREQ)
                        .on_disabled_hover_text(help::SPECIAL_FREQ);
                        if state.config.channel3_policy == ModePolicy::Auto {
                            theme::note(
                                ui,
                                "Channel 3 Auto already searches FNUM/BLOCK; lock the frequencies to fix them.",
                            );
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(theme::caption("Loss weights"));
                            if ui
                                .small_button("Defaults")
                                .on_hover_text("Restore the default weights for the current pitched/unpitched setting.")
                                .clicked()
                            {
                                state.config.weights = if state.config.unpitched {
                                    LossWeights::unpitched()
                                } else {
                                    LossWeights::default()
                                };
                            }
                        });
                        let w = &mut state.config.weights;
                        // Sessions saved before objective v6 lack this weight; show
                        // the value the next run uses and store it only once edited.
                        let saved_inharmonic = w.inharmonic;
                        let mut inharmonic =
                            saved_inharmonic.unwrap_or(DEFAULT_INHARMONIC_WEIGHT);
                        egui::Grid::new("weights")
                            .num_columns(4)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                for (n, (label, value)) in [
                                    ("Pitch", &mut w.pitch),
                                    ("Partials", &mut w.partials),
                                    ("AM periodicity", &mut w.periodicity),
                                    ("Log-STFT", &mut w.log_spectral),
                                    ("Convergence", &mut w.convergence),
                                    ("Envelope", &mut w.envelope),
                                    ("Harmonics", &mut w.harmonic),
                                    ("Transients", &mut w.transient),
                                    ("Modulation", &mut w.modulation),
                                    ("Tail", &mut w.tail),
                                    ("Bands", &mut w.bands),
                                    ("Off-grid", &mut inharmonic),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    ui.label(RichText::new(label).color(theme::MUTED))
                                        .on_hover_text(help::weight(label));
                                    ui.add(
                                        egui::DragValue::new(value)
                                            .range(0.0..=10000.0)
                                            .speed(0.05),
                                    )
                                    .on_hover_text(help::weight(label));
                                    if n % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        if saved_inharmonic.is_some()
                            || inharmonic != DEFAULT_INHARMONIC_WEIGHT
                        {
                            w.inharmonic = Some(inharmonic);
                        }
                    });
                egui::CollapsingHeader::new(theme::caption(&format!(
                    "Parameter locks · {} fixed",
                    state.locks.len()
                )))
                .id_salt("fit-locks")
                .show(ui, |ui| {
                    theme::note(ui, help::LOCKS);
                    ui.horizontal(|ui| {
                        if ui.small_button("Lock all").clicked() {
                            state.locks = all_parameters(true).into_iter().collect();
                        }
                        if ui.small_button("Unlock all").clicked() {
                            state.locks.clear();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for p in [
                            Parameter::Algorithm,
                            Parameter::Feedback,
                            Parameter::Channel3Mode,
                            Parameter::LfoEnabled,
                            Parameter::LfoRate,
                            Parameter::Ams,
                            Parameter::Fms,
                        ] {
                            lock(ui, &mut state.locks, p, &p.label());
                        }
                    });
                    egui::Grid::new("operator-locks")
                        .num_columns(5)
                        .spacing([14.0, 2.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("");
                            for index in 0..4 {
                                ui.label(
                                    RichText::new(format!("S{}", index + 1))
                                        .color(theme::OPERATORS[index])
                                        .strong(),
                                );
                            }
                            ui.end_row();
                            for field in OperatorField::ALL {
                                ui.label(RichText::new(field.label()).monospace().color(theme::MUTED));
                                for index in 0..4 {
                                    lock(ui, &mut state.locks, Parameter::Operator { index, field }, "");
                                }
                                ui.end_row();
                            }
                        });
                    if special {
                        ui.horizontal_wrapped(|ui| {
                            for index in 0..3 {
                                for p in [
                                    Parameter::SpecialFnum { index },
                                    Parameter::SpecialBlock { index },
                                ] {
                                    lock(ui, &mut state.locks, p, &p.label());
                                }
                            }
                        });
                    }
                });
            });
        });
    }
    fn fit_run(&mut self, ui: &mut egui::Ui, running: bool, ready: bool) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::title(ui, "Run", "");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(ready && !running, theme::primary("▶ Start search"))
                    .on_hover_text(help::START_FIT)
                    .on_disabled_hover_text(if running {
                        "A search is already running."
                    } else {
                        "Load a reference first."
                    })
                    .clicked()
                {
                    self.start_fit(false);
                }
                if ui
                    .add_enabled(running, egui::Button::new("Cancel"))
                    .on_hover_text("Stop the search; finished candidates are kept.")
                    .clicked()
                {
                    self.fit.cancel(false);
                }
            });
            theme::note(
                ui,
                "Uses the region, pitch and note markers from the reference panel. Lower loss is closer; the editor is never changed until you apply a candidate.",
            );
            if let Some(p) = &self.fit.progress {
                let elapsed = self
                    .fit
                    .started
                    .map_or(p.elapsed_seconds, |at| at.elapsed().as_secs_f64());
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(p.evaluations as f32 / p.budget.max(1) as f32)
                        .show_percentage()
                        .fill(theme::ACCENT.gamma_multiply(0.7))
                        .text(p.stage.label())
                        .animate(running),
                );
                theme::stats(
                    ui,
                    "fit-progress",
                    &[
                        (
                            "Evaluations",
                            format!("{} / {}", p.evaluations, p.budget),
                            help::BUDGET,
                        ),
                        (
                            "Renders",
                            p.renders.to_string(),
                            "Real chip renders (cache hits are not re-rendered).",
                        ),
                        ("Generation", p.generation.to_string(), ""),
                        ("Elapsed", format!("{elapsed:.1} s"), ""),
                        (
                            "Current / best",
                            format!(
                                "{} / {}",
                                p.current.map_or("—".into(), |v| format!("{v:.5}")),
                                p.best
                                    .as_ref()
                                    .map_or("—".into(), |c| format!("{:.5}", c.loss.total))
                            ),
                            "Loss of the latest and the best candidate so far.",
                        ),
                    ],
                );
            }
            if !self.fit.message.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&self.fit.message).color(theme::ACCENT));
            }
        });
    }
    fn fit_results(&mut self, ui: &mut egui::Ui, running: bool, ready: bool) {
        let Some(state) = self.session.fitting.as_ref().filter(|s| !s.runs.is_empty()) else {
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::title(ui, "Results", "");
                theme::note(
                    ui,
                    "Ranked candidates and the loss history appear here after a search. The search is stochastic and bounded by its budget; it does not guarantee recovering the original registers.",
                );
            });
            return;
        };
        self.fit.run = self.fit.run.min(state.runs.len().saturating_sub(1));
        let old_run = self.fit.run;
        let mut apply = None;
        let mut refine = false;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Results").size(15.0).strong());
                egui::ComboBox::from_id_salt("fitting-run")
                    .width(300.0)
                    .selected_text(format!(
                        "Run {} of {} · saved in session",
                        self.fit.run + 1,
                        state.runs.len()
                    ))
                    .show_ui(ui, |ui| {
                        for (index, run) in state.runs.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.fit.run,
                                index,
                                format!(
                                    "Run {} · {:?} · seed {} · best {:.5}",
                                    index + 1,
                                    run.status,
                                    run.config.seed,
                                    run.candidates.first().map_or(run.initial_loss.total, |c| c.loss.total)
                                ),
                            );
                        }
                    });
            });
            if self.fit.run != old_run {
                self.fit.candidate = 0;
            }
            let Some(run) = state.runs.get(self.fit.run) else {
                return;
            };
            theme::note(
                ui,
                &format!(
                    "Target {} · {:?} / {:?} · note {} · {:.3} s · onset {:.3} s · shared gain ×{:.3} · objective v{}",
                    run.context.fingerprint,
                    run.context.audition.profile,
                    run.context.audition.clock,
                    run.context.audition.note,
                    run.context.audition.total_seconds,
                    run.context.reference_onset_seconds,
                    run.context.reference_gain,
                    run.context.loss_version
                ),
            );
            theme::note(
                ui,
                &format!(
                    "{} · {} · {} cache hits · spectral CPU {:.3} ms / GPU {} ms{}",
                    run.performance.backend,
                    run.performance.adapter.as_deref().unwrap_or("no GPU"),
                    run.performance.cache_hits,
                    run.performance.cpu_fft_ms,
                    run.performance
                        .gpu_fft_ms
                        .map_or("—".into(), |v| format!("{v:.3}")),
                    if run.performance.note.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", run.performance.note)
                    }
                ),
            );
            ui.add_space(6.0);
            ui.columns(2, |columns| {
                let ui = &mut columns[0];
                ui.label(theme::caption("Ranked candidates"))
                    .on_hover_text("Scores belong to this run's context. Refining re-scores against the current reference and settings.");
                let worst = run
                    .candidates
                    .iter()
                    .map(|c| c.loss.total)
                    .fold(run.initial_loss.total, f64::max)
                    .max(1e-9);
                egui::Grid::new("ranked-candidates")
                    .num_columns(5)
                    .spacing([10.0, 3.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for label in ["", "Loss", "", "Alg / FB", "Stage"] {
                            ui.label(RichText::new(label).small().color(theme::FAINT));
                        }
                        ui.end_row();
                        for (index, candidate) in run.candidates.iter().enumerate() {
                            ui.selectable_value(&mut self.fit.candidate, index, format!("#{}", index + 1));
                            ui.monospace(format!("{:.5}", candidate.loss.total));
                            ui.add(
                                egui::ProgressBar::new((candidate.loss.total / worst) as f32)
                                    .desired_width(70.0)
                                    .desired_height(5.0)
                                    .fill(theme::ACCENT.gamma_multiply(0.6)),
                            )
                            .on_hover_text(format!(
                                "Relative to the initial patch loss {:.5}",
                                run.initial_loss.total
                            ));
                            ui.monospace(format!(
                                "{} / {}",
                                candidate.instrument.channel.algorithm.display_number(),
                                candidate.instrument.channel.feedback.get()
                            ));
                            ui.label(RichText::new(candidate.stage.label()).small().color(theme::MUTED));
                            ui.end_row();
                        }
                    });
                self.fit.candidate = self
                    .fit
                    .candidate
                    .min(run.candidates.len().saturating_sub(1));
                ui.add_space(4.0);
                ui.label(theme::caption("Best loss per evaluation"));
                let history = run
                    .history
                    .iter()
                    .map(|p| [p.evaluation as f64, p.best])
                    .collect::<Vec<_>>();
                Chart::new(110.0, [0.0, run.evaluations.max(1) as f64], [0.0, run.initial_loss.total.max(1e-6)])
                    .units(Unit::Count, Unit::Loss)
                    .show(ui, &[Series::area("best", &history, theme::ACCENT)]);

                let ui = &mut columns[1];
                let Some(selected) = run.candidates.get(self.fit.candidate) else {
                    return;
                };
                let loss = selected.loss;
                ui.label(theme::caption(&format!("Candidate #{}", self.fit.candidate + 1)));
                theme::stats(
                    ui,
                    "candidate-detail",
                    &[
                        (
                            "Recording gain",
                            format!(
                                "×{:.3} ({:+.1} dB) · {}",
                                loss.recording_gain,
                                20.0 * loss.recording_gain.log10(),
                                if run.config.match_level { "compensated" } else { "not compensated" }
                            ),
                            help::RECORDING_GAIN,
                        ),
                        ("Pitch", format!("{:.4}", loss.pitch), help::weight("Pitch")),
                        ("Partials", format!("{:.4}", loss.partials), help::weight("Partials")),
                        ("AM periodicity", format!("{:.4}", loss.periodicity), help::weight("AM periodicity")),
                        ("Log-STFT", format!("{:.4}", loss.log_spectral), help::weight("Log-STFT")),
                        ("Convergence", format!("{:.4}", loss.convergence), help::weight("Convergence")),
                        ("Envelope", format!("{:.4}", loss.envelope), help::weight("Envelope")),
                        ("Harmonics", format!("{:.4}", loss.harmonic), help::weight("Harmonics")),
                        ("Transients", format!("{:.4}", loss.transient), help::weight("Transients")),
                        ("Modulation", format!("{:.4}", loss.modulation), help::weight("Modulation")),
                        ("Tail", format!("{:.4}", loss.tail), help::weight("Tail")),
                        ("Bands", format!("{:.4}", loss.bands), help::weight("Bands")),
                        ("Off-grid", format!("{:.4}", loss.inharmonic), help::weight("Off-grid")),
                        (
                            "Modes",
                            format!(
                                "CH3 {} · LFO {} (rate {}) · AMS {} · FMS {}",
                                if selected.instrument.channel3_special.is_some() { "on" } else { "off" },
                                if selected.instrument.global.lfo_enabled { "on" } else { "off" },
                                selected.instrument.global.lfo_rate.get(),
                                selected.instrument.channel.ams.get(),
                                selected.instrument.channel.fms.get(),
                            ),
                            "",
                        ),
                        (
                            "AM per operator",
                            selected
                                .instrument
                                .operators
                                .iter()
                                .map(|o| if o.am { "on" } else { "off" })
                                .collect::<Vec<_>>()
                                .join(" · "),
                            help::AM,
                        ),
                    ],
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!running, theme::primary("Apply"))
                        .on_hover_text(help::APPLY)
                        .clicked()
                    {
                        apply = Some(false);
                    }
                    if ui
                        .add_enabled(!running, egui::Button::new("▶ Apply & listen"))
                        .on_hover_text(help::APPLY_PLAY)
                        .clicked()
                    {
                        apply = Some(true);
                    }
                    if ui
                        .add_enabled(ready && !running, egui::Button::new("Refine"))
                        .on_hover_text(help::REFINE)
                        .clicked()
                    {
                        refine = true;
                    }
                });
            });
            theme::note(
                ui,
                "Sessions keep up to 8 runs and 256 history points per run, with their patches and locks.",
            );
        });
        if let Some(play) = apply {
            self.apply_candidate(play);
        }
        if refine {
            self.start_fit(true);
        }
    }
}
fn lock(ui: &mut egui::Ui, locks: &mut discofm_fit::ParameterLocks, p: Parameter, label: &str) {
    let mut locked = locks.contains(&p);
    if ui
        .checkbox(&mut locked, label)
        .on_hover_text(format!("Lock {}", p.label()))
        .changed()
    {
        if locked {
            locks.insert(p);
        } else {
            locks.remove(&p);
        }
    }
}
