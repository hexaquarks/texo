use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use texo::compiler::{self, RenderedDocument};

pub struct RenderJob {
    result: Receiver<Result<RenderedDocument>>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    started: Instant,
}

impl RenderJob {
    pub fn start(source: String, path: PathBuf) -> Result<Self> {
        Self::spawn(source, path, false)
    }

    pub fn draft(source: String, path: PathBuf) -> Result<Self> {
        Self::spawn(source, path, true)
    }

    fn spawn(source: String, path: PathBuf, draft: bool) -> Result<Self> {
        let (sender, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::Builder::new()
            .name("texo-render".into())
            .spawn(move || {
                let rendered = if draft {
                    compiler::render_draft(source, &path, &worker_cancelled)
                } else {
                    compiler::render(source, &path, &worker_cancelled)
                };
                let _ = sender.send(rendered);
            })?;

        Ok(Self {
            result,
            cancelled,
            worker: Some(worker),
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn poll(&self) -> Option<Result<RenderedDocument>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(anyhow!("The render worker stopped unexpectedly")))
            }
        }
    }
}

impl Drop for RenderJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
