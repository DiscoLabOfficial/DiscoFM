use crate::{FitError, FitInput, FitOutcome, FitProgress, fit};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

/// A poisoned mailbox still holds valid data; never turn it into a second panic.
fn lock(shared: &Mutex<WorkerSnapshot>) -> std::sync::MutexGuard<'_, WorkerSnapshot> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
#[derive(Default)]
pub struct WorkerSnapshot {
    pub progress: Option<FitProgress>,
    pub result: Option<Result<FitOutcome, FitError>>,
}
/// Single producer, coalesced progress and one terminal result. UI polling and
/// cancellation never wait for queue space. Completed candidates contain no PCM.
pub struct FitWorker {
    shared: Arc<Mutex<WorkerSnapshot>>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl FitWorker {
    pub fn start(input: FitInput) -> Result<Self, FitError> {
        input.validate()?;
        let shared = Arc::new(Mutex::new(WorkerSnapshot::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (output, stop) = (Arc::clone(&shared), Arc::clone(&cancel));
        let handle = thread::Builder::new()
            .name("discofm-fit".into())
            .spawn(move || {
                // A panic must still deliver a terminal result, or the UI would
                // wait forever. The panic hook has already logged the details;
                // release builds abort before reaching this point.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fit(
                        input,
                        || stop.load(Ordering::Acquire),
                        |p| lock(&output).progress = Some(p),
                    )
                }))
                .unwrap_or_else(|payload| {
                    let message = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".into());
                    Err(FitError::Internal(message))
                });
                lock(&output).result = Some(result);
            })
            .map_err(|error| FitError::Internal(format!("cannot start fitting thread: {error}")))?;
        Ok(Self {
            shared,
            cancel,
            handle: Some(handle),
        })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn poll(&self) -> WorkerSnapshot {
        let mut shared = lock(&self.shared);
        WorkerSnapshot {
            progress: shared.progress.take(),
            result: shared.result.take(),
        }
    }
}
impl Drop for FitWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(handle) = self.handle.take() {
            // Drivers can take seconds to create a device. Cancellation must
            // never make the GUI wait for them; the owned mailbox/reference
            // remain alive until the detached worker observes cancellation.
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}
