//! Patch library (a folder of sessions and tracker instruments), clipboard
//! copy/paste and instrument import/export.
use super::*;
use discofm_chip::Instrument;
use discofm_formats::InstrumentFormat;
use service::Job;

/// File extensions the library lists and the importer reads.
pub(crate) const PATCH_EXTENSIONS: [&str; 4] = ["json", "tfi", "vgi", "tyi"];

pub(crate) struct Library {
    pub open: bool,
    pub folder: PathBuf,
    pub entries: Vec<PathBuf>,
    pub listed: bool,
    pub filter: String,
    pub name: String,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            open: false,
            folder: default_folder(),
            entries: Vec::new(),
            listed: false,
            filter: String::new(),
            name: String::new(),
        }
    }
}
fn default_folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("DISCOFM_LIBRARY") {
        return PathBuf::from(folder);
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Documents"))
        .filter(|documents| documents.is_dir())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DiscoFM Patches")
}

/// A patch from text: an instrument object, or a whole session.
pub(crate) fn parse_patch(text: &str) -> Result<Instrument, String> {
    let text = text.trim();
    if let Ok(instrument) = serde_json::from_str::<Instrument>(text) {
        return Ok(instrument);
    }
    match discofm_formats::read_session(text.as_bytes()) {
        Ok(session) => Ok(session.instrument),
        Err(_) => Err("The clipboard does not hold a DiscoFM patch".into()),
    }
}
/// Reads a library or import file: a session (.json) or a tracker instrument.
pub(crate) fn read_patch(path: &std::path::Path) -> Result<Instrument, String> {
    if InstrumentFormat::from_path(path).is_some() {
        return InstrumentFormat::read(path).map_err(|e| e.to_string());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    discofm_formats::read_session(file)
        .map(|session| session.instrument)
        .map_err(|e| e.to_string())
}
pub(crate) fn list(folder: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    PATCH_EXTENSIONS
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(e))
                })
        })
        .collect();
    paths.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(paths)
}
/// Library file names keep letters, digits, spaces and a few separators.
fn file_stem(name: &str) -> Option<String> {
    let stem: String = name
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')'))
        .collect();
    let stem = stem.trim().trim_matches('.').to_owned();
    (!stem.is_empty()).then_some(stem)
}

impl Editor {
    /// Replaces the whole instrument as one undo step.
    pub(super) fn replace_instrument(&mut self, instrument: Instrument, status: String) {
        if instrument == self.session.instrument {
            self.status = status;
            return;
        }
        let before = std::mem::replace(&mut self.session.instrument, instrument);
        self.undo.record(before);
        self.instrument_replaced();
        self.status = status;
    }
    /// Applies an instrument changed outside the widgets (undo, paste, import).
    pub(super) fn instrument_replaced(&mut self) {
        if let Some(channel) = self.session.instrument.required_channel()
            && self.session.audition.channel != channel
        {
            self.session.audition.channel = channel;
            self.stop();
            self.playback = None;
        } else if self.listening == Listening::Live
            && let Some(p) = &mut self.playback
        {
            let result = p.controls().set_instrument(self.session.instrument.clone());
            self.report(result);
        }
        self.dirty();
    }
    pub(super) fn step_history(&mut self, redo: bool) {
        let changed = if redo {
            self.undo.redo(&mut self.session.instrument)
        } else {
            self.undo.undo(&mut self.session.instrument)
        };
        if changed {
            self.instrument_replaced();
            self.status = if redo { "Redone" } else { "Undone" }.into();
        }
    }
    pub(super) fn copy_patch(&mut self, ctx: &egui::Context) {
        match serde_json::to_string_pretty(&self.session.instrument) {
            Ok(text) => {
                ctx.copy_text(text);
                self.status = "Patch copied to the clipboard".into();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub(super) fn paste_patch(&mut self, text: &str) {
        let result = parse_patch(text);
        if let Some(instrument) = self.report(result) {
            self.replace_instrument(instrument, "Patch pasted from the clipboard".into());
        }
    }
    pub(super) fn paste_from_clipboard(&mut self) {
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text());
        if let Some(text) = self.report(text) {
            self.paste_patch(&text);
        }
    }
    pub(super) fn import_patch_dialog(&mut self) {
        self.stop();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Instruments", &PATCH_EXTENSIONS)
            .add_filter("Tracker instruments", &["tfi", "vgi", "tyi"])
            .add_filter("DiscoFM session", &["json"])
            .pick_file()
        {
            let result = self.service.job(Job::Patch(path));
            self.report(result);
        }
    }
    pub(super) fn export_dialog(&mut self) {
        self.stop();
        let stem = self
            .session_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "instrument".into(), |n| n.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new().set_file_name(format!("{stem}.tfi"));
        for format in InstrumentFormat::ALL {
            dialog = dialog.add_filter(
                format!("{} (.{})", format.name(), format.extension()),
                &[format.extension()],
            );
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let Some(format) = InstrumentFormat::from_path(&path) else {
            self.error = Some("Export as .tfi, .vgi or .tyi".into());
            return;
        };
        let omitted = format.omitted(&self.session.instrument);
        let bytes = format.encode(&self.session.instrument);
        let result = self.service.job(Job::Export(path, bytes, true));
        if self.report(result).is_some() {
            self.export_note = (!omitted.is_empty())
                .then(|| format!("{} cannot store: {}", format.name(), omitted.join(", ")));
        }
    }
    fn refresh_library(&mut self) {
        let result = self.service.job(Job::Library(self.library.folder.clone()));
        if self.report(result).is_some() {
            self.library.listed = true;
        }
    }
    fn add_to_library(&mut self) {
        let Some(stem) = file_stem(&self.library.name) else {
            self.error = Some("Name the patch first".into());
            return;
        };
        let session = Session {
            instrument: self.session.instrument.clone(),
            audition: self.session.audition.clone(),
            ..Session::default()
        };
        let mut bytes = Vec::new();
        let result = discofm_formats::write_session(&mut bytes, &session);
        if self.report(result).is_none() {
            return;
        }
        let path = self.library.folder.join(format!("{stem}.json"));
        let result = self.service.job(Job::Export(path, bytes, false));
        if self.report(result).is_some() {
            self.library.name.clear();
        }
    }
    pub(super) fn library_window(&mut self, ctx: &egui::Context) {
        if !self.library.open {
            return;
        }
        if !self.library.listed {
            self.refresh_library();
        }
        let mut open = self.library.open;
        let mut load = None;
        egui::Window::new("Patch library")
            .open(&mut open)
            .default_size([360.0, 460.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(self.library.folder.display().to_string())
                        .small()
                        .color(theme::MUTED),
                )
                .on_hover_text(help::LIBRARY);
                ui.horizontal(|ui| {
                    if ui.button("Choose folder…").clicked()
                        && let Some(folder) = rfd::FileDialog::new()
                            .set_directory(&self.library.folder)
                            .pick_folder()
                    {
                        self.library.folder = folder;
                        self.library.listed = false;
                    }
                    if ui.button("Refresh").clicked() {
                        self.library.listed = false;
                    }
                    if ui.button("Show in Explorer").clicked() {
                        let result = std::fs::create_dir_all(&self.library.folder)
                            .and_then(|()| reveal(&self.library.folder));
                        self.report(result);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.library.name)
                            .hint_text("Name for the current patch")
                            .desired_width(220.0),
                    );
                    if ui
                        .button("Add")
                        .on_hover_text(help::LIBRARY_ADD)
                        .clicked()
                    {
                        self.add_to_library();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.library.filter)
                        .hint_text("Filter")
                        .desired_width(f32::INFINITY),
                );
                let filter = self.library.filter.to_lowercase();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut shown = 0;
                    for path in &self.library.entries {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        shown += 1;
                        if ui
                            .selectable_label(false, &name)
                            .on_hover_text("Load this patch into the editor (undo restores the current one).")
                            .clicked()
                        {
                            load = Some(path.clone());
                        }
                    }
                    if shown == 0 {
                        theme::note(
                            ui,
                            if self.library.entries.is_empty() {
                                "No patches yet. Name the current patch and press Add, or put .json/.tfi/.vgi/.tyi files in this folder."
                            } else {
                                "No patch matches the filter."
                            },
                        );
                    }
                });
            });
        self.library.open = open;
        if let Some(path) = load {
            let result = self.service.job(Job::Patch(path));
            self.report(result);
        }
    }
}
fn reveal(folder: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new(if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    })
    .arg(folder)
    .spawn()
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_parse_from_instrument_or_session_text_and_names_are_sanitised() {
        let mut instrument = Instrument::default();
        instrument.operators[1].mul = discofm_chip::Value::new(7).unwrap();
        let text = serde_json::to_string(&instrument).unwrap();
        assert_eq!(parse_patch(&format!("  {text}\n")).unwrap(), instrument);
        let session = Session {
            instrument: instrument.clone(),
            ..Session::default()
        };
        let mut bytes = Vec::new();
        discofm_formats::write_session(&mut bytes, &session).unwrap();
        assert_eq!(
            parse_patch(std::str::from_utf8(&bytes).unwrap()).unwrap(),
            instrument
        );
        assert!(parse_patch("hello").is_err());
        assert!(parse_patch(&text.replace("\"mul\":7", "\"mul\":16")).is_err());
        assert_eq!(file_stem(" Bass: lead/1 ").as_deref(), Some("Bass lead1"));
        assert_eq!(file_stem("..."), None);
    }

    #[test]
    fn library_lists_only_patch_files_and_reads_each_format() {
        let folder = std::env::temp_dir().join(format!("discofm-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        assert!(list(&folder).unwrap().is_empty());
        std::fs::create_dir_all(&folder).unwrap();
        let instrument = Instrument::default();
        for format in InstrumentFormat::ALL {
            std::fs::write(
                folder.join(format!("b.{}", format.extension())),
                format.encode(&instrument),
            )
            .unwrap();
        }
        std::fs::write(folder.join("notes.txt"), "x").unwrap();
        let entries = list(&folder).unwrap();
        assert_eq!(entries.len(), 3);
        for path in &entries {
            assert_eq!(read_patch(path).unwrap(), instrument);
        }
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
