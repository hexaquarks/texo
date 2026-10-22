use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Result, anyhow};
use texo::compiler::{self, RenderedDocument};

pub struct RenderJob {
    result: Receiver<Result<RenderedDocument>>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RenderJob {
    pub fn start(source: String, path: PathBuf) -> Result<Self> {
        let (sender, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::Builder::new()
            .name("texo-render".into())
            .spawn(move || {
                let rendered = compiler::render(source, &path, &worker_cancelled);
                let _ = sender.send(rendered);
            })?;

        Ok(Self {
            result,
            cancelled,
            worker: Some(worker),
        })
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
