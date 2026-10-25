use std::{
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use secrecy::SecretString;
use texo::auth::{CODEX_STATUS_TIMEOUT, Provider, codex_command};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) type ModelLoader = fn(Provider, SecretString, &AtomicBool) -> Result<Vec<String>>;

pub(super) struct ModelJob {
    pub(super) receiver: mpsc::Receiver<Result<Vec<String>>>,
    cancelled: Arc<AtomicBool>,
}

impl ModelJob {
    pub(super) fn start(provider: Provider, key: SecretString, loader: ModelLoader) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);

        thread::spawn(move || {
            let _ = sender.send(loader(provider, key, &worker_cancelled));
        });

        Self {
            receiver,
            cancelled,
        }
    }
}

impl Drop for ModelJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(super) struct LoginProcess {
    child: Child,
    started: Instant,
    pub(super) status_only: bool,
}

impl LoginProcess {
    #[cfg(test)]
    pub(super) fn for_test(child: Child, started: Instant) -> Self {
        Self {
            child,
            started,
            status_only: true,
        }
    }

    pub(super) fn start(status_only: bool) -> Result<Self> {
        let mut command = codex_command();
        command.arg("login");
        if status_only {
            command.arg("status");
        }

        command
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn().map_err(|_| {
            anyhow!("Codex is not installed. Install it for ChatGPT sign-in, or choose an API key")
        })?;

        Ok(Self {
            child,
            started: Instant::now(),
            status_only,
        })
    }

    pub(super) fn poll(&mut self) -> Option<Result<()>> {
        let timeout = if self.status_only {
            CODEX_STATUS_TIMEOUT
        } else {
            LOGIN_TIMEOUT
        };

        match self.child.try_wait() {
            Ok(Some(status)) => Some(if status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "Sign-in did not complete. Retry, or run `codex login` in your terminal"
                ))
            }),
            Ok(None) if self.started.elapsed() < timeout => None,
            Ok(None) => Some(Err(anyhow!("Sign-in timed out. Use /login to retry"))),
            Err(_) => Some(Err(anyhow!(
                "Could not check sign-in status. Use /login to retry"
            ))),
        }
    }
}

impl Drop for LoginProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
