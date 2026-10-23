//! Desktop instrument editor and reference comparison.
#![forbid(unsafe_code)]
mod banks;
mod compare_view;
mod comparison;
mod controls;
mod fitting;
mod help;
mod library;
mod midi;
mod plots;
mod reference;
mod service;
mod theme;
mod toolbar;
mod undo;

use discofm_analysis::AnalysisOptions;
use discofm_audio::{AudioBuffer, DeviceOptions, Playback, VoiceId};
use discofm_engine::LiveConfig;
use discofm_formats::{ReferenceSettings, Session};
use discofm_soundfont::{SampleSelection, SoundBank};
use eframe::egui::{self, RichText};
use service::{Job, Preview, PreviewInput, ResultEvent, Service};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct LaunchOptions {
    pub session: Option<PathBuf>,
    pub reference: Option<PathBuf>,
    pub reference_sample: Option<SampleSelection>,
    /// Development verification: capture one ready frame, then close normally.
    pub smoke_screenshot: Option<PathBuf>,
}
pub fn run(options: LaunchOptions) -> Result<(), Box<dyn std::error::Error>> {
    let screenshot = options.smoke_screenshot.clone();
    if screenshot.as_ref().is_some_and(|p| p.exists()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "screenshot destination exists",
        )
        .into());
    }
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DiscoFM")
            .with_icon(theme::icon())
            .with_inner_size([1520.0, 1000.0])
            .with_min_inner_size([1200.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DiscoFM",
        native,
        Box::new(move |cc| Ok(Box::new(Editor::new(&cc.egui_ctx, options)))),
    )?;
    if screenshot.is_some_and(|p| !p.exists()) {
        return Err(std::io::Error::other("GUI screenshot did not complete").into());
    }
    Ok(())
}
#[derive(Clone, Copy, PartialEq)]
enum Listening {
    Stopped,
    Live,
    Reference,
    Generated,
}
struct Editor {
    session: Session,
    fit: fitting::FitUi,
    options: AnalysisOptions,
    service: Service,
    revision: usize,
    dirty_at: Option<Instant>,
    preview: Option<Box<Preview>>,
    reference: Option<Arc<AudioBuffer>>,
    bank: Option<Arc<SoundBank>>,
    bank_filter: String,
    pending_bank_selection: Option<SampleSelection>,
    reference_wave: Vec<[f64; 2]>,
    generated_wave: Vec<[f64; 2]>,
    reference_texture: Option<egui::TextureHandle>,
    generated_texture: Option<egui::TextureHandle>,
    reference_id: usize,
    session_id: usize,
    reference_loading: bool,
    session_loading: bool,
    pending_reference_settings: Option<ReferenceSettings>,
    pending_reference_apply: Option<usize>,
    request_reference_apply: bool,
    explicit_reference: bool,
    session_path: Option<PathBuf>,
    playback: Option<Playback>,
    device_id: Option<String>,
    devices: Vec<discofm_audio::DeviceInfo>,
    devices_listed: bool,
    listening: Listening,
    match_ab_level: bool,
    held: Option<(egui::Key, VoiceId)>,
    input_frame: Option<u64>,
    pointer_voice: Option<VoiceId>,
    pointer_semitone: Option<u8>,
    octave: i8,
    note_on_marker: f64,
    key_off_marker: f64,
    manual_hz: f64,
    manual_note: u8,
    tab: usize,
    status: String,
    error: Option<String>,
    /// Last message written to the application log, so each error is logged once.
    logged_error: Option<String>,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    started: Instant,
    undo: undo::Undo,
    /// Undo (false) or redo (true) requested by a widget, applied next frame.
    history_request: Option<bool>,
    library: library::Library,
    midi: midi::Midi,
    midi_ports: Option<Vec<String>>,
    midi_voice: Option<(u8, VoiceId)>,
    /// The session as last opened or saved, for the unsaved-changes check.
    saved: Session,
    pending_save: Option<Box<Session>>,
    export_note: Option<String>,
    close_prompt: bool,
    close_after_save: bool,
    allow_close: bool,
}
impl Editor {
    fn new(ctx: &egui::Context, options: LaunchOptions) -> Self {
        theme::apply(ctx);
        let mut app = Self {
            session: Session::default(),
            fit: fitting::FitUi::default(),
            options: AnalysisOptions::default(),
            service: Service::new(),
            revision: 1,
            dirty_at: Some(Instant::now()),
            preview: None,
            reference: None,
            bank: None,
            bank_filter: String::new(),
            pending_bank_selection: None,
            reference_wave: Vec::new(),
            generated_wave: Vec::new(),
            reference_texture: None,
            generated_texture: None,
            reference_id: 0,
            session_id: 0,
            reference_loading: false,
            session_loading: false,
            pending_reference_settings: None,
            pending_reference_apply: None,
            request_reference_apply: false,
            explicit_reference: options.reference.is_some(),
            session_path: None,
            playback: None,
            device_id: None,
            devices: Vec::new(),
            devices_listed: false,
            listening: Listening::Stopped,
            match_ab_level: true,
            held: None,
            input_frame: None,
            pointer_voice: None,
            pointer_semitone: None,
            octave: 4,
            note_on_marker: 0.0,
            key_off_marker: 0.5,
            manual_hz: 440.0,
            manual_note: 69,
            tab: 0,
            status: "Starting the chip…".into(),
            error: None,
            logged_error: None,
            screenshot: options.smoke_screenshot,
            screenshot_requested: false,
            started: Instant::now(),
            undo: undo::Undo::default(),
            history_request: None,
            library: library::Library::default(),
            midi: midi::Midi::default(),
            midi_ports: None,
            midi_voice: None,
            saved: Session::default(),
            pending_save: None,
            export_note: None,
            close_prompt: false,
            close_after_save: false,
            allow_close: false,
        };
        if let Some(path) = options.session {
            app.load_session(path);
        }
        if let Some(path) = options.reference {
            app.load_reference_selected(path, options.reference_sample);
        }
        app
    }
    fn report<T>(&mut self, result: Result<T, impl std::fmt::Display>) -> Option<T> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }
    fn stop(&mut self) {
        self.fit.play_revision = None;
        if let Some(p) = &mut self.playback {
            p.controls().stop();
        }
        self.held = None;
        self.pointer_voice = None;
        self.listening = Listening::Stopped;
    }
    fn ensure_playback(&mut self) -> bool {
        if self.playback.is_none() {
            let r = &self.session.audition;
            let result = Playback::open(
                self.session.instrument.clone(),
                LiveConfig {
                    channel: r.channel,
                    profile: r.profile,
                    clock: r.clock,
                    tuning_cents: r.tuning_cents,
                },
                DeviceOptions {
                    device_id: self.device_id.clone(),
                    ..Default::default()
                },
            );
            self.playback = self.report(result);
        }
        self.playback.is_some()
    }
    fn trigger(&mut self, note: u8) -> Option<VoiceId> {
        self.trigger_velocity(note, self.session.audition.velocity)
    }
    fn trigger_velocity(
        &mut self,
        note: u8,
        velocity: discofm_chip::Value<127>,
    ) -> Option<VoiceId> {
        self.fit.play_revision = None;
        if !self.ensure_playback() {
            return None;
        }
        let p = self.playback.as_mut()?;
        let result = p
            .controls()
            .set_instrument(self.session.instrument.clone())
            .and_then(|()| p.controls().set_profile(self.session.audition.profile))
            .and_then(|()| p.controls().note_on(note, velocity));
        let id = self.report(result)?;
        self.listening = Listening::Live;
        Some(id)
    }
    fn release(&mut self) {
        self.fit.play_revision = None;
        if let Some(p) = &mut self.playback {
            p.controls().release();
        }
        self.held = None;
        self.pointer_voice = None;
    }
    fn play_clip(&mut self, reference: bool) {
        self.fit.play_revision = None;
        if self.reference_loading || self.session_loading {
            return;
        }
        let audio = self
            .preview
            .as_ref()
            .filter(|p| p.revision == self.revision)
            .and_then(|p| {
                p.comparison
                    .as_ref()
                    .map(|c| c.clip(reference))
                    .or_else(|| (!reference).then(|| Arc::clone(&p.audio)))
            });
        if let Some(audio) = audio
            && self.ensure_playback()
            && let Some(playback) = self.playback.as_mut()
        {
            let result = playback.controls().play_clip(audio);
            if self.report(result).is_some() {
                self.listening = if reference {
                    Listening::Reference
                } else {
                    Listening::Generated
                };
                self.held = None;
                self.pointer_voice = None;
            }
        }
    }
    fn dirty(&mut self) {
        self.pending_reference_apply = None;
        self.fit.cancel(false);
        self.error = None;
        self.revision = self.service.invalidate();
        self.dirty_at = Some(Instant::now());
        if matches!(self.listening, Listening::Reference | Listening::Generated) {
            self.stop();
        }
    }
    fn apply_reference_pitch(&mut self, duration: f64, pitch: Option<f64>) -> bool {
        let duration = self
            .options
            .end_seconds
            .unwrap_or((self.options.start_seconds + 30.0).min(duration));
        if self.key_off_marker <= self.note_on_marker || duration <= self.note_on_marker {
            self.error = Some("Key-off must come after note-on".into());
            return false;
        }
        let Some(hz) = pitch else {
            self.error = Some("Analysis found no stable pitch. For hits, noise or inharmonic sounds, select a tonal region or set a manual pitch (Hz or note).".into());
            return false;
        };
        if duration - self.note_on_marker > 30.0 || self.key_off_marker > duration {
            self.error =
                Some("Use at most 30 s after note-on, with key-off inside the region".into());
            return false;
        }
        self.session.audition.key_on_seconds = self.key_off_marker - self.note_on_marker;
        self.session.audition.total_seconds = duration - self.note_on_marker;
        self.session.audition.note = (69.0 + 12.0 * (hz / 440.0).log2())
            .round()
            .clamp(0.0, 119.0) as u8;
        self.session.audition.tuning_cents = 1200.0
            * (hz / discofm_engine::midi_hz(self.session.audition.note).expect("bounded note"))
                .log2();
        self.stop();
        self.playback = None;
        self.error = None;
        self.status = format!(
            "Applied {hz:.2} Hz → {} · note-on {:.3} s · key-off {:.3} s · length {:.3} s",
            discofm_analysis::note_name(i32::from(self.session.audition.note)),
            self.note_on_marker,
            self.key_off_marker,
            self.session.audition.total_seconds
        );
        true
    }
    fn analyze_and_apply_reference(&mut self) {
        if self.pending_reference_apply.is_some() || self.reference_loading || self.session_loading
        {
            return;
        }
        let Some(audio) = self.reference.clone() else {
            return;
        };
        if self.report(self.options.validate()).is_none() {
            return;
        }
        let result = self
            .service
            .job(Job::Analyze(self.revision, audio, self.options.clone()));
        if self.report(result).is_some() {
            self.pending_reference_apply = Some(self.revision);
            self.error = None;
            self.status = "Analysing reference pitch…".into();
        }
    }
    fn load_reference(&mut self, path: PathBuf) {
        self.load_reference_selected(path, None);
    }
    fn load_reference_selected(&mut self, path: PathBuf, selection: Option<SampleSelection>) {
        self.pending_reference_apply = None;
        self.stop();
        let id = self.reference_id.wrapping_add(1);
        let bank = discofm_soundfont::is_bank(&path);
        let result = self.service.job(if bank {
            Job::Bank(id, path)
        } else {
            Job::Reference(id, path)
        });
        if self.report(result).is_some() {
            self.fit.cancel(false);
            self.pending_reference_settings = None;
            self.reference_id = id;
            self.reference_loading = true;
            self.pending_bank_selection = selection;
            self.status = if bank {
                "Reading sound bank catalogue…"
            } else {
                "Reading audio file…"
            }
            .into();
        }
    }
    fn load_session(&mut self, path: PathBuf) {
        self.pending_reference_apply = None;
        self.stop();
        let id = self.session_id.wrapping_add(1);
        let result = self.service.job(Job::Session(id, path));
        if self.report(result).is_some() {
            self.fit.cancel(true);
            self.reference_id = self.reference_id.wrapping_add(1);
            self.service.cancel_reference(self.reference_id);
            self.reference_loading = false;
            self.pending_bank_selection = None;
            self.pending_reference_settings = None;
            self.session_id = id;
            self.session_loading = true;
            self.status = "Opening session…".into();
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_fit();
        self.poll_midi();
        while let Ok(event) = self.service.results.try_recv() {
            match event {
                ResultEvent::Analyzed(id, result)
                    if self.pending_reference_apply == Some(id) && id == self.revision =>
                {
                    self.pending_reference_apply = None;
                    if let Some(analysis) = self.report(result) {
                        let pitch = self.options.pitch_override_hz.or_else(|| {
                            analysis
                                .pitch
                                .as_ref()
                                .filter(|p| p.confidence >= 0.5)
                                .map(|p| p.hz)
                        });
                        if self.apply_reference_pitch(analysis.original_seconds, pitch) {
                            self.dirty();
                        } else {
                            self.status = "Analysis finished; check the pitch or markers".into();
                        }
                    } else {
                        self.status = "Analysis stopped; see the error".into();
                    }
                }
                ResultEvent::Bank(id, result) if id == self.reference_id => {
                    self.reference_loading = false;
                    if let Some(bank) = self.report(result) {
                        let selection =
                            self.pending_bank_selection
                                .take()
                                .map(Ok)
                                .unwrap_or_else(|| {
                                    let index = bank
                                        .samples
                                        .iter()
                                        .position(|s| s.warning.is_none())
                                        .unwrap_or(0);
                                    bank.selection(index)
                                });
                        self.bank = Some(Arc::clone(&bank));
                        self.bank_filter.clear();
                        if let Some(selection) = self.report(selection) {
                            self.load_bank_sample(bank, selection);
                        }
                    }
                }
                ResultEvent::Reference(id, path, selection, result) if id == self.reference_id => {
                    self.reference_loading = false;
                    if let Some(audio) = self.report(result) {
                        self.reference_wave = plots::waveform(&audio);
                        self.session.reference_path = Some(path.to_string_lossy().into_owned());
                        if selection.is_none() {
                            self.bank = None;
                            self.bank_filter.clear();
                        }
                        self.session.reference_sample = selection;
                        self.reference = Some(Arc::clone(&audio));
                        self.options = AnalysisOptions::default();
                        let duration = audio.frames.len() as f64 / f64::from(audio.sample_rate);
                        if duration > 30.0 {
                            self.options.end_seconds = Some(30.0);
                        }
                        self.note_on_marker = 0.0;
                        self.key_off_marker = duration.min(30.0);
                        if let Some(settings) = self.pending_reference_settings.take() {
                            self.options = settings.options;
                            self.note_on_marker = settings.note_on_seconds;
                            self.key_off_marker = settings.key_off_seconds;
                            // A session's relative reference path now reads as absolute.
                            self.saved.reference_path = self.session.reference_path.clone();
                            self.saved.reference_sample = self.session.reference_sample.clone();
                        }
                        self.manual_hz = self.options.pitch_override_hz.unwrap_or(440.0);
                        self.manual_note = (69.0 + 12.0 * (self.manual_hz / 440.0).log2())
                            .round()
                            .clamp(16.0, 107.0) as u8;
                        self.reference_texture = None;
                        self.status = "Reference loaded".into();
                        self.error = None;
                        self.dirty();
                    }
                }
                ResultEvent::Session(id, path, result) if id == self.session_id => {
                    self.session_loading = false;
                    if let Some(session) = self.report(result) {
                        self.stop();
                        self.playback = None;
                        self.session = *session;
                        self.fit.run = self
                            .session
                            .fitting
                            .as_ref()
                            .map_or(0, |s| s.runs.len().saturating_sub(1));
                        self.fit.candidate = 0;
                        self.fit.progress = None;
                        self.fit.view = self
                            .session
                            .fitting
                            .as_ref()
                            .is_some_and(|s| !s.runs.is_empty());
                        self.session_path = Some(path.clone());
                        self.saved = self.session.clone();
                        self.undo.clear();
                        self.error = None;
                        self.dirty();
                        if !self.explicit_reference {
                            self.reference = None;
                            self.bank = None;
                            self.reference_wave.clear();
                            self.reference_texture = None;
                            if let Some(reference) = &self.session.reference_path {
                                let reference = PathBuf::from(reference);
                                let selection = self.session.reference_sample.clone();
                                self.load_reference_selected(
                                    if reference.is_absolute() {
                                        reference
                                    } else {
                                        path.parent()
                                            .unwrap_or(std::path::Path::new("."))
                                            .join(reference)
                                    },
                                    selection,
                                );
                                self.pending_reference_settings =
                                    self.session.reference_analysis.clone();
                            }
                        }
                        self.status = "Session opened".into();
                    }
                }
                ResultEvent::Saved(result) => {
                    let saved = self.pending_save.take();
                    if let Some(path) = self.report(result) {
                        self.session_path = Some(path);
                        self.status = "Session saved".into();
                        if let Some(session) = saved {
                            self.saved = *session;
                        }
                        if self.close_after_save {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    self.close_after_save = false;
                }
                ResultEvent::Exported(result) => {
                    if let Some(path) = self.report(result) {
                        self.status = format!("Saved {}", path.display());
                        if let Some(note) = self.export_note.take() {
                            self.status = format!("{} · {note}", self.status);
                        }
                        if path.parent() == Some(self.library.folder.as_path()) {
                            self.library.listed = false;
                        }
                    }
                }
                ResultEvent::Patch(path, result) => {
                    if let Some(instrument) = self.report(result) {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.replace_instrument(*instrument, format!("Loaded patch {name}"));
                    }
                }
                ResultEvent::Library(folder, result) => {
                    if folder == self.library.folder
                        && let Some(entries) = self.report(result)
                    {
                        self.library.entries = entries;
                    }
                }
                ResultEvent::Preview(id, result) if id == self.revision => {
                    if let Some(preview) = self.report(result) {
                        self.generated_texture = Some(plots::texture(
                            ctx,
                            "generated",
                            &preview.generated.spectrogram,
                        ));
                        self.reference_texture = preview
                            .comparison
                            .as_ref()
                            .map(|c| &c.analysis)
                            .map(|a| plots::texture(ctx, "reference", &a.spectrogram));
                        let mut preview = preview;
                        self.generated_wave = std::mem::take(&mut preview.curves.generated_wave);
                        if preview.comparison.is_some() {
                            self.reference_wave =
                                std::mem::take(&mut preview.curves.reference_wave);
                        }
                        self.preview = Some(preview);
                        self.status = "Ready".into();
                        if self.fit.play_revision == Some(id) {
                            self.fit.play_revision = None;
                            self.play_clip(false);
                        }
                    }
                }
                _ => {}
            }
        }
        if self
            .dirty_at
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(150))
        {
            self.dirty_at = None;
            match self
                .session
                .validate()
                .map_err(|e| e.to_string())
                .and_then(|()| self.options.validate().map_err(|e| e.to_string()))
            {
                Ok(()) => {
                    self.service.submit(PreviewInput {
                        revision: self.revision,
                        session: self.session.clone(),
                        reference: self.reference.clone(),
                        options: self.options.clone(),
                        note_on_seconds: self.note_on_marker,
                        match_level: self.match_ab_level,
                    });
                    self.status = "Rendering preview…".into();
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(p) = &mut self.playback {
            if let Err(error) = p.check_health() {
                self.error = Some(error.to_string());
                self.stop();
                self.playback = None;
            } else if matches!(self.listening, Listening::Reference | Listening::Generated)
                && p.controls().clip_finished()
            {
                self.listening = Listening::Stopped;
            }
        }
        if self.error != self.logged_error {
            if let Some(error) = &self.error {
                discofm_log::error("editor", error);
            }
            self.logged_error = self.error.clone();
        }
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        if self.input_frame == Some(frame) {
            return;
        }
        self.input_frame = Some(frame);
        let events = ctx.input(|i| i.events.clone());
        let editing = ctx.egui_wants_keyboard_input();
        for event in events {
            match event {
                egui::Event::WindowFocused(false) => self.stop(),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.stop(),
                egui::Event::Key {
                    key,
                    pressed: false,
                    ..
                } => {
                    if let Some((held, id)) = self.held
                        && key == held
                    {
                        if let Some(p) = &mut self.playback {
                            p.controls().note_off(id);
                        }
                        self.held = None;
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !editing && modifiers.command => match key {
                    egui::Key::Z => self.step_history(modifiers.shift),
                    egui::Key::Y => self.step_history(true),
                    egui::Key::S => {
                        self.save_dialog();
                    }
                    _ => {}
                },
                egui::Event::Copy if !editing => self.copy_patch(ctx),
                egui::Event::Paste(text) if !editing => self.paste_patch(&text),
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !editing && !modifiers.alt => {
                    if key == egui::Key::Escape {
                        self.stop();
                    } else if key == egui::Key::Space {
                        self.release();
                    } else if let Some(semitone) = piano_keys().iter().position(|k| *k == key)
                        && let Some(note) = self.report(discofm_audio::octave_note(
                            self.octave,
                            semitone as u8,
                            self.session.audition.clock,
                        ))
                        && let Some(id) = self.trigger(note)
                    {
                        self.held = Some((key, id));
                    }
                }
                egui::Event::Screenshot { image, .. } => {
                    if let Some(path) = self.screenshot.take() {
                        let result = image::save_buffer(
                            &path,
                            image.as_raw(),
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        );
                        if self.report(result).is_some() {
                            println!("GUI screenshot: {}", path.display());
                        }
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                _ => {}
            }
        }
        if !ctx.input(|i| i.pointer.primary_down())
            && let Some(id) = self.pointer_voice.take()
            && let Some(p) = &mut self.playback
        {
            p.controls().note_off(id);
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped.first() {
            self.explicit_reference = true;
            self.load_reference(file.path().to_path_buf());
        }
    }
    fn poll_midi(&mut self) {
        for event in self.midi.drain() {
            match event {
                midi::MidiEvent::On { note, velocity } => {
                    let Ok(velocity) = discofm_chip::Value::new(velocity) else {
                        continue;
                    };
                    if let Some(id) = self.trigger_velocity(note, velocity) {
                        self.midi_voice = Some((note, id));
                    }
                }
                midi::MidiEvent::Off { note } => {
                    if let Some((held, id)) = self.midi_voice
                        && held == note
                    {
                        if let Some(p) = &mut self.playback {
                            p.controls().note_off(id);
                        }
                        self.midi_voice = None;
                    }
                }
                midi::MidiEvent::AllOff => {
                    self.release();
                    self.midi_voice = None;
                }
            }
        }
    }
    /// Differs from the file last opened or saved (or from a new session).
    fn unsaved(&self) -> bool {
        let saved = &self.saved;
        let reference = self.reference.as_ref().map(|_| ReferenceSettings {
            options: self.options.clone(),
            note_on_seconds: self.note_on_marker,
            key_off_seconds: self.key_off_marker,
        });
        self.session.instrument != saved.instrument
            || self.session.audition != saved.audition
            || self.session.reference_path != saved.reference_path
            || self.session.reference_sample != saved.reference_sample
            || self.session.fitting != saved.fitting
            || (self.reference.is_some() && reference != saved.reference_analysis)
    }
    /// Asks before closing over unsaved work; Save closes once the file is written.
    fn close_dialog(&mut self, ctx: &egui::Context) {
        if !self.close_prompt {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("close-prompt")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Save changes?");
            ui.label("This session has changes that are not saved. Closing discards them.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary("Save…")).clicked() {
                    choice = Some(0);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(2);
                }
            });
        });
        match choice {
            Some(0) => {
                self.close_prompt = false;
                self.close_after_save = self.save_dialog();
            }
            Some(1) => {
                self.close_prompt = false;
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(_) => self.close_prompt = false,
            None => {}
        }
    }
    /// Something is in flight that will change what is on screen soon.
    fn busy(&self) -> bool {
        self.reference_loading
            || self.session_loading
            || self.dirty_at.is_some()
            || self.pending_reference_apply.is_some()
            || self.fit.worker.is_some()
            || self.listening != Listening::Stopped
            || self
                .preview
                .as_ref()
                .is_none_or(|p| p.revision != self.revision)
    }
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let spinner = self.busy() && self.listening == Listening::Stopped;
        let mut dismiss = false;
        egui::Sides::new().shrink_left().truncate().show(
            ui,
            |ui| {
                if spinner {
                    ui.add(egui::Spinner::new().size(12.0));
                }
                ui.label(RichText::new(&self.status).color(theme::MUTED));
                if let Some(error) = &self.error {
                    dismiss = ui
                        .small_button("✖")
                        .on_hover_text("Dismiss this message")
                        .clicked();
                    let first = error.lines().next().unwrap_or_default();
                    ui.add(
                        egui::Label::new(RichText::new(format!("⚠ {first}")).color(theme::ERROR))
                            .truncate(),
                    )
                    .on_hover_text(format!(
                        "{error}

Saved in {}",
                        discofm_log::log_path().display()
                    ));
                    if ui
                        .small_button("Log")
                        .on_hover_text(help::OPEN_LOG)
                        .clicked()
                    {
                        open_log_folder();
                    }
                }
            },
            |ui| {
                let (text, color) = match self.listening {
                    Listening::Stopped => ("idle", theme::FAINT),
                    Listening::Live => ("● live note", theme::GOOD),
                    Listening::Reference => ("▶ A · reference", theme::REFERENCE),
                    Listening::Generated => ("▶ B · chip", theme::ACCENT),
                };
                theme::badge(ui, text, color);
                if let Some(p) = &self.playback {
                    ui.label(
                        RichText::new(format!(
                            "{} Hz · {} ch · {} underrun frames",
                            p.info().sample_rate,
                            p.info().channels,
                            p.stats().underrun_frames
                        ))
                        .small()
                        .color(theme::FAINT),
                    )
                    .on_hover_text(
                        "Output stream format and frames the device had to fill with silence.",
                    );
                } else {
                    ui.label(
                        RichText::new("audio opens on first play")
                            .small()
                            .color(theme::FAINT),
                    );
                }
            },
        );
        if dismiss {
            self.error = None;
        }
    }
    fn drop_overlay(&self, ui: &egui::Ui, area: egui::Rect) {
        if !ui.input(|i| !i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop-overlay"),
        ));
        painter.rect_filled(area, 0, egui::Color32::from_black_alpha(170));
        let target = area.shrink(40.0);
        painter.rect_stroke(
            target,
            12,
            egui::Stroke::new(2.0, theme::REFERENCE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            target.center(),
            egui::Align2::CENTER_CENTER,
            "Drop to load as reference\nWAV · SF2 · SF3 · SFZ · DLS",
            egui::FontId::proportional(22.0),
            theme::TEXT,
        );
    }
    fn view(&mut self, ui: &mut egui::Ui) {
        if let Some(redo) = self.history_request.take() {
            self.step_history(redo);
        }
        let before_instrument = self.session.instrument.clone();
        let before_audition = self.session.audition.clone();
        let before_revision = self.revision;
        let before_markers = (self.note_on_marker, self.key_off_marker);
        let old_options = self.options.clone();
        let old_match_level = self.match_ab_level;
        let area = ui.max_rect();
        let bar = egui::Frame::new()
            .fill(theme::SURFACE)
            .inner_margin(egui::Margin::symmetric(14, 8));
        egui::Panel::top("header")
            .frame(bar)
            .show(ui, |ui| self.header(ui));
        egui::Panel::top("transport")
            .frame(bar.fill(theme::BG))
            .show(ui, |ui| self.transport(ui));
        egui::Panel::bottom("status")
            .frame(bar.inner_margin(egui::Margin::symmetric(14, 4)))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("reference")
            .resizable(true)
            .default_size(400.0)
            .size_range(340.0..=620.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("reference-scroll")
                    .show(ui, |ui| self.reference_panel(ui));
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("main").show(ui, |ui| {
                    if self.fit.view {
                        self.fitting_panel(ui);
                    } else {
                        controls::instrument(
                            ui,
                            &mut self.session,
                            self.preview
                                .as_deref()
                                .filter(|p| p.revision == self.revision),
                        );
                        ui.add_space(10.0);
                        self.comparison(ui);
                    }
                });
            });
        self.drop_overlay(ui, area);
        let settled = !ui.input(|i| i.pointer.any_down()) && ui.memory(|m| m.focused().is_none());
        self.undo
            .track(&before_instrument, &self.session.instrument, settled);
        if before_instrument != self.session.instrument
            || before_audition != self.session.audition
            || old_options != self.options
            || before_markers != (self.note_on_marker, self.key_off_marker)
            || old_match_level != self.match_ab_level
        {
            if before_audition.channel != self.session.audition.channel
                || before_audition.clock != self.session.audition.clock
                || before_audition.tuning_cents != self.session.audition.tuning_cents
            {
                self.stop();
                self.playback = None;
            } else if self.listening == Listening::Live
                && (before_instrument != self.session.instrument
                    || before_audition.profile != self.session.audition.profile)
                && let Some(p) = &mut self.playback
            {
                let result = p
                    .controls()
                    .set_instrument(self.session.instrument.clone())
                    .and_then(|()| p.controls().set_profile(self.session.audition.profile));
                self.report(result);
            }
            if self.revision == before_revision {
                self.dirty();
            }
        }
        if std::mem::take(&mut self.request_reference_apply) {
            self.analyze_and_apply_reference();
        }
    }
}
impl eframe::App for Editor {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.screenshot.is_none()
            && self.unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.stop();
            self.close_prompt = true;
        }
        // Worker results arrive without input events, so keep polling; slowly when idle.
        ctx.request_repaint_after(Duration::from_millis(if self.busy() { 30 } else { 250 }));
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keyboard(&ctx);
        self.view(ui);
        self.library_window(&ctx);
        self.close_dialog(&ctx);
        if self.screenshot.is_some()
            && !self.screenshot_requested
            && !self.reference_loading
            && !self.session_loading
            && self.error.is_none()
            && self
                .preview
                .as_ref()
                .is_some_and(|p| p.revision == self.revision)
        {
            // Wait for startup file jobs as well as the current analysis revision.
            if self.started.elapsed() > Duration::from_secs(2) && self.status == "Ready" {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }
        if self.screenshot.is_some() && self.started.elapsed() > Duration::from_secs(60) {
            eprintln!("GUI smoke timeout: {:?}", self.error);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
fn open_log_folder() {
    if let Err(error) = discofm_log::reveal() {
        discofm_log::warn("editor", format!("cannot open the log folder: {error}"));
    }
}
fn piano_keys() -> [egui::Key; 13] {
    use egui::Key::*;
    [Z, S, X, D, C, V, G, B, H, N, J, M, Comma]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn frame(ctx: &egui::Context, app: &mut Editor, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1460.0, 1100.0),
            )),
            events,
            ..Default::default()
        };
        let mut result = ctx.run_ui(input, |ui| {
            app.keyboard(ui.ctx());
            app.view(ui);
        });
        assert!(!result.shapes.is_empty());
        result.textures_delta.clear(); // Headless test intentionally has no GPU renderer.
    }
    fn ready(ctx: &egui::Context, app: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.poll(ctx);
            if !app.reference_loading
                && !app.session_loading
                && app
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.revision == app.revision)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("preview not ready: {:?}", app.error);
    }
    #[test]
    fn headless_editor_renders_every_comparison_and_focus_loss_stops_without_opening_a_device() {
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        app.session.audition.key_on_seconds = 0.0001;
        app.session.audition.total_seconds = 0.0002;
        let short = app.session.clone();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.session, short,
            "display must preserve valid sub-millisecond timings"
        );
        ready(&ctx, &mut app);
        for tab in 0..4 {
            app.tab = tab;
            frame(&ctx, &mut app, vec![]);
        }
        app.listening = Listening::Live;
        frame(&ctx, &mut app, vec![egui::Event::WindowFocused(false)]);
        assert!(app.listening == Listening::Stopped);
        assert!(app.playback.is_none());
        let old = app.revision;
        app.session.instrument.operators[3].ar = discofm_chip::Value::new(7).unwrap();
        app.dirty();
        assert_ne!(old, app.revision);
        ready(&ctx, &mut app);
        assert_eq!(
            app.preview
                .as_ref()
                .unwrap()
                .render
                .envelopes
                .last()
                .unwrap()
                .operators[3]
                .registers
                .ar
                .get(),
            7
        );
    }
    #[test]
    fn paste_undo_redo_and_unsaved_state_without_opening_audio() {
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        frame(&ctx, &mut app, vec![]);
        assert!(!app.unsaved());
        let original = app.session.instrument.clone();
        let mut pasted = original.clone();
        pasted.operators[2].mul = discofm_chip::Value::new(9).unwrap();
        // Press and release, or egui reports the next press as a repeat.
        let shortcut = |key, shift| {
            let modifiers = egui::Modifiers {
                ctrl: true,
                command: true,
                shift,
                ..Default::default()
            };
            [true, false]
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                })
                .to_vec()
        };
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Paste(serde_json::to_string(&pasted).unwrap())],
        );
        assert_eq!(app.session.instrument, pasted);
        assert!(app.unsaved());
        frame(&ctx, &mut app, shortcut(egui::Key::Z, false));
        assert_eq!(app.session.instrument, original);
        assert!(!app.unsaved());
        frame(&ctx, &mut app, shortcut(egui::Key::Z, true));
        assert_eq!(app.session.instrument, pasted);
        frame(&ctx, &mut app, shortcut(egui::Key::Z, false));
        frame(&ctx, &mut app, shortcut(egui::Key::Y, false));
        assert_eq!(app.session.instrument, pasted);
        // Ctrl+Z is a shortcut, never the piano's Z key.
        assert!(app.playback.is_none());
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Paste("not a patch".into())],
        );
        assert!(app.error.is_some());
        assert_eq!(app.session.instrument, pasted);
    }
    #[test]
    fn prepared_clips_stop_on_context_edits_and_cannot_start_during_file_replacement() {
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.session.audition.total_seconds = 0.1;
        app.session.audition.key_on_seconds = 0.1;
        app.reference = Some(Arc::new(
            discofm_audio::from_native(
                &discofm_engine::render_note(&app.session.instrument, &app.session.audition)
                    .unwrap(),
                48000,
            )
            .unwrap(),
        ));
        app.dirty();
        ready(&ctx, &mut app);
        assert!(app.preview.as_ref().unwrap().comparison.is_some());
        app.reference_loading = true;
        app.play_clip(true);
        app.play_clip(false);
        assert!(
            app.playback.is_none(),
            "pending import must not open or start audio"
        );
        app.reference_loading = false;
        app.session_loading = true;
        app.play_clip(true);
        app.play_clip(false);
        assert!(
            app.playback.is_none(),
            "pending session must not start old clips"
        );
        app.session_loading = false;
        for listening in [Listening::Reference, Listening::Generated] {
            app.listening = listening;
            app.dirty();
            assert!(app.listening == Listening::Stopped);
        }
    }
    #[test]
    fn fitting_ui_runs_without_mutating_editor_applies_selected_and_cancels_on_context_changes() {
        use discofm_fit::{
            FitConfig, FitMode, FittingState, OperatorField, Parameter, all_parameters,
        };
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.session.audition.key_on_seconds = 0.06;
        app.session.audition.total_seconds = 0.12;
        let mut target = app.session.instrument.clone();
        target.operators[3].tl = discofm_chip::Value::new(48).unwrap();
        app.reference = Some(Arc::new(
            discofm_audio::from_native(
                &discofm_engine::render_note(&target, &app.session.audition).unwrap(),
                24000,
            )
            .unwrap(),
        ));
        app.reference_wave = plots::waveform(app.reference.as_ref().unwrap());
        app.options.trim_silence = false;
        app.options.pitch_override_hz = Some(440.0);
        app.key_off_marker = 0.06;
        app.session.fitting = Some(FittingState {
            config: FitConfig {
                mode: FitMode::Basic,
                compute: discofm_fit::ComputeMode::Cpu,
                match_level: false,
                evaluations: 96,
                population: 8,
                ..Default::default()
            },
            locks: all_parameters(false)
                .into_iter()
                .filter(|p| {
                    *p != Parameter::Operator {
                        index: 3,
                        field: OperatorField::Tl,
                    }
                })
                .collect(),
            runs: Vec::new(),
        });
        app.dirty();
        ready(&ctx, &mut app);
        let initial = app.session.instrument.clone();
        let revision = app.revision;
        app.start_fit(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.fit.worker.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            frame(&ctx, &mut app, vec![]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.fit.worker.is_none(),
            "fitting did not finish: {:?}",
            app.error
        );
        assert_eq!(
            app.session.instrument, initial,
            "search must not apply candidates implicitly"
        );
        assert_eq!(
            app.revision, revision,
            "fitting metadata must not invalidate previews or cancel itself"
        );
        let run = &app.session.fitting.as_ref().unwrap().runs[0];
        assert!(run.candidates[0].loss.total < run.initial_loss.total);
        let selected = run.candidates[0].instrument.clone();
        frame(&ctx, &mut app, vec![]);
        app.apply_candidate(false);
        assert_eq!(app.session.instrument, selected);
        assert!(app.playback.is_none());
        ready(&ctx, &mut app);
        app.apply_candidate(true);
        assert_eq!(app.fit.play_revision, Some(app.revision));
        app.stop();
        ready(&ctx, &mut app);
        assert!(app.fit.play_revision.is_none());
        assert!(
            app.playback.is_none(),
            "a delayed preview must never restart after Stop"
        );
        app.session.fitting.as_mut().unwrap().config.evaluations = 20000;
        app.start_fit(true);
        app.session.audition.note = 70;
        app.dirty();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.fit.worker.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.fit.worker.is_none());
        assert_eq!(app.session.audition.note, 70);
        assert_eq!(app.session.instrument, selected);
        app.start_fit(true);
        let path =
            std::env::temp_dir().join(format!("discofm-fit-replace-{}.json", std::process::id()));
        discofm_formats::write_session(fs::File::create_new(&path).unwrap(), &Session::default())
            .unwrap();
        app.load_session(path.clone());
        ready(&ctx, &mut app);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.fit.worker.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.session.fitting.is_none(),
            "an old run must not be appended to a new session"
        );
        drop(app);
        fs::remove_file(path).unwrap();
    }
    #[test]
    #[ignore = "slow GUI stress probe; run in release with --nocapture"]
    fn repeated_large_refits_through_the_editor() {
        use discofm_fit::{ComputeMode, FitConfig, FittingState};
        let count = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.session.audition.key_on_seconds = 0.2;
        app.session.audition.total_seconds = 0.3;
        let mut target = app.session.instrument.clone();
        target.operators[3].tl = discofm_chip::Value::new(30).expect("tl");
        target.channel.feedback = discofm_chip::Value::new(5).expect("feedback");
        let native = discofm_engine::render_note(&target, &app.session.audition).expect("render");
        let reference = Arc::new(discofm_audio::from_native(&native, 24000).expect("resample"));
        app.reference_wave = plots::waveform(&reference);
        app.reference = Some(reference);
        app.options.trim_silence = false;
        app.options.pitch_override_hz = Some(440.0);
        app.key_off_marker = 0.2;
        app.session.fitting = Some(FittingState {
            config: FitConfig {
                evaluations: count("DISCOFM_STRESS_EVALUATIONS", 6000),
                population: count("DISCOFM_STRESS_POPULATION", 64),
                candidates: 16,
                compute: match std::env::var("DISCOFM_STRESS_COMPUTE").as_deref() {
                    Ok("cpu") => ComputeMode::Cpu,
                    Ok("gpu") => ComputeMode::Gpu,
                    _ => ComputeMode::Auto,
                },
                ..Default::default()
            },
            ..Default::default()
        });
        app.dirty();
        ready(&ctx, &mut app);
        for round in 0..count("DISCOFM_STRESS_ROUNDS", 12) {
            app.start_fit(round > 0);
            assert!(app.fit.worker.is_some(), "round {round}: {:?}", app.error);
            let started = Instant::now();
            while app.fit.worker.is_some() {
                app.poll(&ctx);
                frame(&ctx, &mut app, vec![]);
                std::thread::sleep(Duration::from_millis(15));
            }
            let state = app.session.fitting.as_ref().expect("fitting state");
            println!(
                "round {round}: {:.1} s, {} runs kept, selected run {} candidate {}, {} | {:?}",
                started.elapsed().as_secs_f64(),
                state.runs.len(),
                app.fit.run,
                app.fit.candidate,
                app.fit.message,
                app.error
            );
            assert!(app.error.is_none(), "round {round}: {:?}", app.error);
            // Like a user browsing results: pick the last candidate, apply it.
            app.fit.candidate = 15;
            frame(&ctx, &mut app, vec![]);
            app.apply_candidate(false);
            ready(&ctx, &mut app);
            frame(&ctx, &mut app, vec![]);
            let state = app.session.fitting.as_ref().expect("fitting state");
            let mut json = Vec::new();
            discofm_formats::write_session(&mut json, &app.session).expect("session write");
            println!(
                "   session {} KiB, {} runs",
                json.len() / 1024,
                state.runs.len()
            );
        }
    }
    #[test]
    fn reference_apply_button_runs_analysis_applies_pitch_and_times_and_reports_no_pitch() {
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.reference = Some(Arc::new(AudioBuffer {
            sample_rate: 8000,
            frames: (0..2400)
                .map(|i| [(i as f32 * std::f32::consts::TAU * 440.0 / 8000.0).sin() * 0.2; 2])
                .collect(),
        }));
        app.note_on_marker = 0.05;
        app.key_off_marker = 0.2;
        app.session.audition.note = 60;
        app.analyze_and_apply_reference();
        assert!(app.pending_reference_apply.is_some());
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.pending_reference_apply.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.pending_reference_apply.is_none());
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.session.audition.note, 69);
        assert!((app.session.audition.key_on_seconds - 0.15).abs() < 1e-10);
        assert!((app.session.audition.total_seconds - 0.25).abs() < 1e-10);
        assert!(app.playback.is_none());
        let previous = app.session.audition.clone();
        app.reference = Some(Arc::new(AudioBuffer {
            sample_rate: 8000,
            frames: vec![[0.0; 2]; 2400],
        }));
        app.dirty();
        app.analyze_and_apply_reference();
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.pending_reference_apply.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.pending_reference_apply.is_none());
        assert!(app.error.as_ref().unwrap().contains("no stable pitch"));
        assert_eq!(app.session.audition, previous);
        app.options.pitch_override_hz = Some(330.0);
        app.dirty();
        app.analyze_and_apply_reference();
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.pending_reference_apply.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.session.audition.note, 64);
        app.analyze_and_apply_reference();
        app.session.audition.note = 70;
        app.dirty();
        ready(&ctx, &mut app);
        assert_eq!(
            app.session.audition.note, 70,
            "editing invalidates pending application"
        );
        app.options.start_seconds = 0.3;
        app.options.end_seconds = None;
        app.dirty();
        app.analyze_and_apply_reference();
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.pending_reference_apply.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.pending_reference_apply.is_none());
        assert!(app.error.is_some());
        assert!(!app.status.contains("Analysing"));
        app.options.start_seconds = 0.0;
        app.dirty();
        app.analyze_and_apply_reference();
        app.load_reference(PathBuf::from(format!(
            "target/missing-reference-{}.wav",
            std::process::id()
        )));
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.reference_loading && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.pending_reference_apply.is_none());
        assert_eq!(
            app.session.audition.note, 70,
            "old analysis cannot apply after reference replacement"
        );
    }
    #[test]
    fn session_relative_reference_loads_original_pcm_and_invalid_import_remains_visible() {
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        let folder = std::env::temp_dir().join(format!("discofm-ui-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("referência com espaços.wav");
        let session_path = folder.join("session.json");
        let audio = AudioBuffer {
            sample_rate: 8000,
            frames: (0..4000)
                .map(|i| {
                    let x = (i as f32 * std::f32::consts::TAU * 220.0 / 8000.0).sin() * 0.2;
                    [x, x + 0.1]
                })
                .collect(),
        };
        discofm_audio::write_wav(&path, &audio).unwrap();
        let session = Session {
            reference_path: Some("referência com espaços.wav".into()),
            reference_analysis: Some(ReferenceSettings {
                options: AnalysisOptions {
                    fft_size: 4096,
                    end_seconds: Some(0.45),
                    normalize: true,
                    pitch_override_hz: Some(1000.0),
                    ..Default::default()
                },
                note_on_seconds: 0.05,
                key_off_seconds: 0.4,
            }),
            ..Session::default()
        };
        discofm_formats::write_session(fs::File::create_new(&session_path).unwrap(), &session)
            .unwrap();
        app.load_session(session_path);
        ready(&ctx, &mut app);
        assert_eq!(app.reference.as_deref(), Some(&audio));
        assert_eq!(
            app.options,
            session.reference_analysis.as_ref().unwrap().options
        );
        assert_eq!(app.note_on_marker, 0.05);
        assert_eq!(app.key_off_marker, 0.4);
        assert_eq!(app.manual_hz, 1000.0);
        assert_eq!(app.manual_note, 83);
        assert!(app.preview.as_ref().unwrap().reference.is_some());
        let old_note = app.session.audition.note;
        app.options.pitch_override_hz = None;
        app.dirty();
        app.apply_reference_pitch(0.5, None);
        assert_eq!(app.session.audition.note, old_note);
        assert!(app.error.is_some(), "stale pitch must not be applied");
        app.options.pitch_override_hz = Some(440.0);
        app.apply_reference_pitch(0.5, Some(440.0));
        assert_eq!(app.session.audition.note, 69);
        app.options.pitch_override_hz = None;
        app.load_reference(folder.join("missing.wav"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.reference_loading && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_some());
        assert_eq!(app.reference.as_deref(), Some(&audio));
        for _ in 0..4 {
            frame(&ctx, &mut app, vec![]);
            app.poll(&ctx);
        }
        assert!(app.error.is_some());
        drop(app);
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn dropped_reference_after_session_request_wins_over_automatic_session_reference() {
        #[derive(Debug)]
        struct DropFile(PathBuf);
        impl egui::DroppedFile for DropFile {
            fn path(&self) -> &std::path::Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                fs::read(&self.0).map_err(|e| e.to_string())
            }
        }
        let folder = std::env::temp_dir().join(format!("discofm-drop-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let wave = folder.join("later.wav");
        let source = AudioBuffer {
            sample_rate: 8000,
            frames: vec![[0.2, 0.3]; 400],
        };
        discofm_audio::write_wav(&wave, &source).unwrap();
        let path = folder.join("session.json");
        let session = Session {
            reference_path: Some("missing-old.wav".into()),
            ..Default::default()
        };
        discofm_formats::write_session(fs::File::create_new(&path).unwrap(), &session).unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.load_session(path);
        let mut result = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1460.0, 1000.0),
                )),
                dropped_files: vec![Arc::new(DropFile(wave.clone()))],
                ..Default::default()
            },
            |ui| app.keyboard(ui.ctx()),
        );
        result.textures_delta.clear();
        ready(&ctx, &mut app);
        assert!(app.error.is_none());
        assert_eq!(app.reference.as_deref(), Some(&source));
        assert_eq!(
            std::path::Path::new(app.session.reference_path.as_ref().unwrap())
                .canonicalize()
                .unwrap(),
            wave.canonicalize().unwrap()
        );
        drop(app);
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn soundfont_selection_analyzes_stops_switches_and_restores_relative_session() {
        let folder = std::env::temp_dir().join(format!("discofm-bank-ui-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("bank.sfz");
        let first = AudioBuffer {
            sample_rate: 8000,
            frames: (0..1600)
                .map(|n| [(n as f32 * 0.2).sin() * 0.2; 2])
                .collect(),
        };
        let second = AudioBuffer {
            sample_rate: 8000,
            frames: (0..1600)
                .map(|n| [(n as f32 * 0.4).sin() * 0.3; 2])
                .collect(),
        };
        discofm_audio::write_wav(folder.join("one.wav"), &first).unwrap();
        discofm_audio::write_wav(folder.join("two.wav"), &second).unwrap();
        fs::write(&path, "<region> sample=one.wav\n<region> sample=two.wav").unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.load_reference(path.clone());
        ready(&ctx, &mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.bank.as_ref().unwrap().samples.len(), 2);
        assert_eq!(app.reference.as_deref(), Some(&first));
        let bank = app.bank.clone().unwrap();
        app.listening = Listening::Reference;
        app.load_bank_sample(bank.clone(), bank.selection(1).unwrap());
        assert!(app.listening == Listening::Stopped);
        ready(&ctx, &mut app);
        assert_eq!(app.reference.as_deref(), Some(&second));
        assert_eq!(app.session.reference_sample.as_ref().unwrap().index, 1);
        assert!(app.preview.as_ref().unwrap().comparison.is_some());
        frame(&ctx, &mut app, vec![]);
        let mut saved = app.session.clone();
        saved.reference_path = Some("bank.sfz".into());
        saved.reference_analysis = Some(ReferenceSettings {
            options: AnalysisOptions {
                fft_size: 8192,
                ..Default::default()
            },
            note_on_seconds: 0.01,
            key_off_seconds: 0.15,
        });
        let session_path = folder.join("selected.json");
        discofm_formats::write_session(fs::File::create_new(&session_path).unwrap(), &saved)
            .unwrap();
        app.explicit_reference = false;
        app.load_session(session_path);
        ready(&ctx, &mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.reference.as_deref(), Some(&second));
        assert_eq!(app.options.fft_size, 8192);
        assert_eq!(app.note_on_marker, 0.01);
        app.load_reference(folder.join("one.wav"));
        ready(&ctx, &mut app);
        assert!(app.session.reference_sample.is_none());
        assert!(app.bank.is_none());
        assert_eq!(app.reference.as_deref(), Some(&first));
        drop(app);
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn opening_empty_session_cancels_pending_soundfont_catalog_and_sample() {
        let folder = std::env::temp_dir().join(format!("discofm-bank-race-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let bank = folder.join("bank.sfz");
        fs::write(&bank, "<region> sample=missing.wav").unwrap();
        let path = folder.join("empty.json");
        discofm_formats::write_session(fs::File::create_new(&path).unwrap(), &Session::default())
            .unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::new(&ctx, LaunchOptions::default());
        app.load_reference(bank);
        app.explicit_reference = false;
        app.load_session(path);
        ready(&ctx, &mut app);
        assert!(app.reference.is_none());
        assert!(app.bank.is_none());
        assert!(app.session.reference_sample.is_none());
        assert!(app.session.reference_path.is_none());
        assert!(app.error.is_none(), "{:?}", app.error);
        drop(app);
        fs::remove_dir_all(folder).unwrap();
    }
}
