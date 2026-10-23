use crate::plots::{Curves, waveform};
use discofm_analysis::{Analysis, AnalysisOptions, analyze_cancellable};
use discofm_audio::{AudioBuffer, StreamingResampler};
use discofm_engine::{DiagnosticRender, render_diagnostics};
use discofm_formats::{Session, read_session, write_session};
use discofm_soundfont::{SampleSelection, SoundBank, read_reference};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) struct PreviewInput {
    pub revision: usize,
    pub session: Session,
    pub reference: Option<Arc<AudioBuffer>>,
    pub options: AnalysisOptions,
    pub note_on_seconds: f64,
    pub match_level: bool,
}
pub(crate) struct Preview {
    pub revision: usize,
    pub render: DiagnosticRender,
    pub audio: Arc<AudioBuffer>,
    pub generated: Analysis,
    pub reference: Option<Arc<Analysis>>,
    pub comparison: Option<crate::comparison::Comparison>,
    pub curves: PreviewCurves,
}
/// Plot series prepared on the worker so frames only borrow them.
#[derive(Default)]
pub(crate) struct PreviewCurves {
    pub generated: Curves,
    pub reference: Option<Curves>,
    pub prepared_reference: Option<Curves>,
    /// Per operator: raw generator attenuation and final output attenuation.
    pub operators: [[Vec<[f64; 2]>; 2]; 4],
    pub reference_wave: Vec<[f64; 2]>,
    pub generated_wave: Vec<[f64; 2]>,
}
pub(crate) enum Job {
    Reference(usize, PathBuf),
    Bank(usize, PathBuf),
    Sample(usize, Arc<SoundBank>, SampleSelection),
    Session(usize, PathBuf),
    Save(PathBuf, Box<Session>),
    Analyze(usize, Arc<AudioBuffer>, AnalysisOptions),
    /// Writes finished bytes; `false` refuses to replace an existing file.
    Export(PathBuf, Vec<u8>, bool),
    /// Reads an instrument from a session or tracker instrument file.
    Patch(PathBuf),
    Library(PathBuf),
}
pub(crate) enum ResultEvent {
    Reference(
        usize,
        PathBuf,
        Option<SampleSelection>,
        Result<Arc<AudioBuffer>, String>,
    ),
    Bank(usize, Result<Arc<SoundBank>, String>),
    Session(usize, PathBuf, Result<Box<Session>, String>),
    Saved(Result<PathBuf, String>),
    Exported(Result<PathBuf, String>),
    Patch(PathBuf, Result<Box<discofm_chip::Instrument>, String>),
    Library(PathBuf, Result<Vec<PathBuf>, String>),
    Preview(usize, Result<Box<Preview>, String>),
    Analyzed(usize, Result<Arc<Analysis>, String>),
}
pub(crate) struct Service {
    pub revision: Arc<AtomicUsize>,
    reference_request: Arc<Mutex<usize>>,
    pending: Arc<Mutex<Option<PreviewInput>>>,
    jobs: SyncSender<Job>,
    pub results: Receiver<ResultEvent>,
    quit: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Service {
    pub fn new() -> Self {
        let revision = Arc::new(AtomicUsize::new(1));
        let reference_request = Arc::new(Mutex::new(0));
        let request = Arc::clone(&reference_request);
        let pending = Arc::new(Mutex::new(None::<PreviewInput>));
        let quit = Arc::new(AtomicBool::new(false));
        let (jobs, receiver) = mpsc::sync_channel(4);
        let (sender, results) = mpsc::sync_channel(4);
        let (r, p, q) = (
            Arc::clone(&revision),
            Arc::clone(&pending),
            Arc::clone(&quit),
        );
        let worker = thread::spawn(move || {
            let mut cache: Option<(Arc<AudioBuffer>, AnalysisOptions, Arc<Analysis>)> = None;
            while !q.load(Ordering::Acquire) {
                if let Ok(job) = receiver.try_recv() {
                    let event = run_job(job, &r, &q, &request);
                    send(&sender, event, &q);
                    continue;
                }
                let input = p.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(input) = input {
                    let revision = input.revision;
                    let cancel =
                        || q.load(Ordering::Acquire) || r.load(Ordering::Acquire) != revision;
                    let result = build_preview(input, &mut cache, &cancel);
                    if !cancel() {
                        send(
                            &sender,
                            ResultEvent::Preview(revision, result.map(Box::new)),
                            &q,
                        );
                    }
                } else {
                    thread::sleep(Duration::from_millis(15));
                }
            }
        });
        Self {
            revision,
            reference_request,
            pending,
            jobs,
            results,
            quit,
            worker: Some(worker),
        }
    }
    pub fn cancel_reference(&self, id: usize) {
        *self
            .reference_request
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = id;
    }
    pub fn invalidate(&self) -> usize {
        self.revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }
    pub fn submit(&self, input: PreviewInput) {
        *self.pending.lock().unwrap_or_else(|p| p.into_inner()) = Some(input);
    }
    pub fn job(&self, job: Job) -> Result<(), String> {
        let id = match &job {
            Job::Reference(id, _) | Job::Bank(id, _) | Job::Sample(id, _, _) => Some(*id),
            _ => None,
        };
        // Hold publication until try_send succeeds. A rejected job cannot cancel
        // its predecessor; the worker cannot start before its ID is published.
        let mut request = self
            .reference_request
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.jobs
            .try_send(job)
            .map_err(|_| "File queue is busy; try again".to_owned())?;
        if let Some(id) = id {
            *request = id;
        }
        Ok(())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Release);
        self.invalidate();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn send(sender: &SyncSender<ResultEvent>, mut event: ResultEvent, quit: &AtomicBool) {
    while !quit.load(Ordering::Acquire) {
        match sender.try_send(event) {
            Ok(()) => return,
            Err(mpsc::TrySendError::Full(value)) => {
                event = value;
                thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return,
        }
    }
}
fn run_job(
    job: Job,
    revision: &AtomicUsize,
    quit: &AtomicBool,
    request: &Mutex<usize>,
) -> ResultEvent {
    match job {
        Job::Reference(id, path) => {
            let path = path.canonicalize().unwrap_or(path);
            let cancel = || {
                quit.load(Ordering::Acquire)
                    || *request.lock().unwrap_or_else(|p| p.into_inner()) != id
            };
            let result = read_reference(&path, None, &cancel)
                .map(Arc::new)
                .map_err(|e| e.to_string());
            ResultEvent::Reference(id, path, None, result)
        }
        Job::Bank(id, path) => {
            let cancel = || {
                quit.load(Ordering::Acquire)
                    || *request.lock().unwrap_or_else(|p| p.into_inner()) != id
            };
            ResultEvent::Bank(
                id,
                SoundBank::open(path, &cancel)
                    .map(Arc::new)
                    .map_err(|e| e.to_string()),
            )
        }
        Job::Sample(id, bank, selection) => {
            let cancel = || {
                quit.load(Ordering::Acquire)
                    || *request.lock().unwrap_or_else(|p| p.into_inner()) != id
            };
            let result = bank
                .read_selection(&selection, &cancel)
                .map(Arc::new)
                .map_err(|e| e.to_string());
            ResultEvent::Reference(id, bank.path.clone(), Some(selection), result)
        }
        Job::Session(id, path) => {
            let result = File::open(&path)
                .map_err(|e| e.to_string())
                .and_then(|f| read_session(f).map(Box::new).map_err(|e| e.to_string()));
            ResultEvent::Session(id, path, result)
        }
        Job::Save(path, session) => ResultEvent::Saved(save(&path, &session).map(|()| path)),
        Job::Export(path, bytes, overwrite) => {
            ResultEvent::Exported(export(&path, &bytes, overwrite).map(|()| path))
        }
        Job::Patch(path) => {
            let result = crate::library::read_patch(&path).map(Box::new);
            ResultEvent::Patch(path, result)
        }
        Job::Library(folder) => {
            let result = crate::library::list(&folder);
            ResultEvent::Library(folder, result)
        }
        Job::Analyze(id, audio, options) => ResultEvent::Analyzed(
            id,
            analyze_cancellable(&audio, &options, || {
                quit.load(Ordering::Acquire) || revision.load(Ordering::Acquire) != id
            })
            .map(Arc::new)
            .map_err(|e| e.to_string()),
        ),
    }
}
fn save(path: &Path, session: &Session) -> Result<(), String> {
    if !path
        .extension()
        .is_some_and(|v| v.eq_ignore_ascii_case("json"))
    {
        return Err("Save the session with a .json extension".into());
    }
    let mut bytes = Vec::new();
    write_session(&mut bytes, session).map_err(|e| e.to_string())?;
    if let Some(reference) = &session.reference_path
        && path
            .canonicalize()
            .ok()
            .zip(Path::new(reference).canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
    {
        return Err("A session cannot overwrite its reference audio".into());
    }
    write_atomic(path, &bytes)
}
fn export(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    if !overwrite {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        let mut file = File::create_new(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!("{} already exists", path.display())
            } else {
                e.to_string()
            }
        })?;
        return file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string());
    }
    write_atomic(path, bytes)
}
/// Replaces `path` only once the new contents are complete on disk.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let extension = path
        .extension()
        .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    let temp = path.with_extension(format!("{extension}.{}.tmp", std::process::id()));
    let mut file = File::create_new(&temp).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| e.to_string())
}
type ReferenceCache = Option<(Arc<AudioBuffer>, AnalysisOptions, Arc<Analysis>)>;
fn build_preview(
    input: PreviewInput,
    cache: &mut ReferenceCache,
    cancel: &impl Fn() -> bool,
) -> Result<Preview, String> {
    let reference = if let Some(audio) = &input.reference {
        if let Some((_, _, analysis)) = cache
            .as_ref()
            .filter(|(old, options, _)| Arc::ptr_eq(old, audio) && options == &input.options)
        {
            Some(Arc::clone(analysis))
        } else {
            let analysis = Arc::new(
                analyze_cancellable(audio, &input.options, cancel).map_err(|e| e.to_string())?,
            );
            *cache = Some((
                Arc::clone(audio),
                input.options.clone(),
                Arc::clone(&analysis),
            ));
            Some(analysis)
        }
    } else {
        *cache = None;
        None
    };
    let render = render_diagnostics(
        &input.session.instrument,
        &input.session.audition,
        1024,
        cancel,
    )
    .map_err(|e| e.to_string())?;
    let mut converter =
        StreamingResampler::new(render.audio.sample_rate, 48000).map_err(|e| e.to_string())?;
    let mut frames = Vec::with_capacity(
        (render.audio.frames.len() as f64 * 48000.0 / render.audio.sample_rate).round() as usize,
    );
    let mut output = [[0.0; 2]; 512];
    for block in render.audio.frames.chunks(512) {
        if cancel() {
            return Err("cancelled".into());
        }
        converter.push(block).map_err(|e| e.to_string())?;
        loop {
            let n = converter.read_into(&mut output);
            frames.extend_from_slice(&output[..n]);
            if n == 0 {
                break;
            }
        }
    }
    converter.finish();
    loop {
        if cancel() {
            return Err("cancelled".into());
        }
        let n = converter.read_into(&mut output);
        frames.extend_from_slice(&output[..n]);
        if n == 0 {
            break;
        }
    }
    let audio = Arc::new(AudioBuffer {
        frames,
        sample_rate: 48000,
    });
    let options = AnalysisOptions {
        trim_silence: false,
        fft_size: input.options.fft_size,
        ..AnalysisOptions::default()
    };
    let (comparison, generated) =
        if let Some((source, analysis)) = input.reference.as_ref().zip(reference.as_ref()) {
            let (comparison, generated) = crate::comparison::build(
                source,
                &audio,
                &input.options,
                analysis,
                input.note_on_seconds,
                input.match_level,
                cancel,
            )?;
            (Some(comparison), generated)
        } else {
            (
                None,
                analyze_cancellable(&audio, &options, cancel).map_err(|e| e.to_string())?,
            )
        };
    let rate = render.audio.sample_rate;
    let operators = std::array::from_fn(|i| {
        let curve = |pick: fn(&discofm_engine::OperatorSnapshot) -> u16| {
            render
                .envelopes
                .iter()
                .map(|v| [v.frame as f64 / rate, f64::from(pick(&v.operators[i]))])
                .collect()
        };
        [
            curve(|o| o.envelope_attenuation),
            curve(|o| o.output_attenuation),
        ]
    });
    let curves = PreviewCurves {
        generated: Curves::new(&generated),
        reference: reference.as_deref().map(Curves::new),
        prepared_reference: comparison.as_ref().map(|c| Curves::new(&c.analysis)),
        operators,
        reference_wave: comparison
            .as_ref()
            .map(|c| waveform(&c.reference))
            .unwrap_or_default(),
        generated_wave: waveform(comparison.as_ref().map_or(&*audio, |c| &*c.generated)),
    };
    Ok(Preview {
        revision: input.revision,
        render,
        audio,
        generated,
        reference,
        comparison,
        curves,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_cache_retains_reference_identity_and_cancellation_discards_old_work() {
        let reference = Arc::new(AudioBuffer {
            sample_rate: 8000,
            frames: (0..1600)
                .map(|i| {
                    let v = (i as f32 * 0.2).sin() * 0.2;
                    [v; 2]
                })
                .collect(),
        });
        let original = reference.clone();
        let mut cache = None;
        let input = |revision| PreviewInput {
            revision,
            session: Session {
                audition: discofm_engine::RenderRequest {
                    total_seconds: 0.05,
                    key_on_seconds: 0.02,
                    ..Default::default()
                },
                ..Default::default()
            },
            reference: Some(reference.clone()),
            options: AnalysisOptions::default(),
            note_on_seconds: 0.0,
            match_level: true,
        };
        let a = build_preview(input(1), &mut cache, &|| false).unwrap();
        let b = build_preview(input(2), &mut cache, &|| false).unwrap();
        assert!(Arc::ptr_eq(
            a.reference.as_ref().unwrap(),
            b.reference.as_ref().unwrap()
        ));
        assert_eq!(*reference, *original);
        let count = std::cell::Cell::new(0);
        assert!(
            build_preview(input(3), &mut cache, &|| {
                count.set(count.get() + 1);
                count.get() > 5
            })
            .is_err()
        );
        let service = Service::new();
        let revision = service.invalidate();
        service.submit(input(1));
        service.submit(input(revision));
        let event = service
            .results
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(event,ResultEvent::Preview(id,Ok(_)) if id==revision));
        drop(service);
    }
    #[test]
    fn session_save_replaces_valid_sessions_and_protects_reference_and_foreign_temp_files() {
        let dir = std::env::temp_dir().join(format!("discofm-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        save(&path, &Session::default()).unwrap();
        let mut session = Session::default();
        session.audition.note = 60;
        save(&path, &session).unwrap();
        assert_eq!(
            read_session(File::open(&path).unwrap())
                .unwrap()
                .audition
                .note,
            60
        );
        let before = fs::read(&path).unwrap();
        session.reference_path = Some(path.to_string_lossy().into_owned());
        assert!(save(&path, &session).is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temp, b"foreign").unwrap();
        session.reference_path = None;
        assert!(save(&path, &session).is_err());
        assert_eq!(fs::read(&temp).unwrap(), b"foreign");
        assert_eq!(before, fs::read(&path).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejected_file_job_preserves_the_active_reference_request() {
        let (jobs, receiver) = mpsc::sync_channel(1);
        let (_, results) = mpsc::sync_channel(1);
        let service = Service {
            revision: Arc::new(AtomicUsize::new(1)),
            reference_request: Arc::new(Mutex::new(7)),
            pending: Arc::new(Mutex::new(None)),
            jobs,
            results,
            quit: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        service
            .job(Job::Session(1, PathBuf::from("queued.json")))
            .unwrap();
        assert!(
            service
                .job(Job::Bank(8, PathBuf::from("rejected.sf2")))
                .is_err()
        );
        assert_eq!(*service.reference_request.lock().unwrap(), 7);
        drop(receiver.recv().unwrap());
        service
            .job(Job::Bank(8, PathBuf::from("accepted.sf2")))
            .unwrap();
        assert_eq!(*service.reference_request.lock().unwrap(), 8);
    }
}
