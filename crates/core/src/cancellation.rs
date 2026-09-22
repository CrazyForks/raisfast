//! Job cancellation.
//!
//! The worker runner registers a per-job cancel channel before executing a
//! handler and races the handler future against it (cooperative soft cancel:
//! the future is dropped at its next await point). CPU-bound work that runs
//! out-of-process (the builtin document parse) reads the task-local
//! [`JOB_CANCEL`] and hard-kills its child process.
//!
//! State is process-global: the worker runner and the admin API live in the
//! same binary. A multi-process deployment would need shared state instead.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tokio::sync::watch;

tokio::task_local! {
    /// Cancel signal for the job currently executing in this task. Set by the
    /// runner around the handler future; read by out-of-process executors.
    pub static JOB_CANCEL: watch::Receiver<bool>;
}

/// Registry of in-flight jobs → cancel channel. `cancel` is idempotent.
#[derive(Default)]
pub struct JobCancels {
    inner: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl JobCancels {
    /// Registers `id` and returns a receiver the runner can race against.
    /// Re-registering an id replaces the previous channel.
    pub fn register(&self, id: &str) -> watch::Receiver<bool> {
        self.register_many(std::slice::from_ref(&id.to_string()))
    }

    /// Registers several job ids onto one shared channel (coalesced jobs:
    /// cancelling any one of them interrupts the merged run).
    pub fn register_many(&self, ids: &[String]) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        if let Ok(mut map) = self.inner.lock() {
            for id in ids {
                map.insert(id.clone(), tx.clone());
            }
        }
        rx
    }

    /// Signals cancellation. Returns `true` if the job was running in this
    /// process (token found).
    pub fn cancel(&self, id: &str) -> bool {
        match self.inner.lock() {
            Ok(map) => match map.get(id) {
                Some(tx) => {
                    let _ = tx.send(true);
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }

    /// Removes the registration once the job reaches a terminal state.
    pub fn finish(&self, ids: &[String]) {
        if let Ok(mut map) = self.inner.lock() {
            for id in ids {
                map.remove(id);
            }
        }
    }
}

/// Process-wide registry shared by the runner and the admin API.
pub static JOB_CANCELS: LazyLock<JobCancels> = LazyLock::new(JobCancels::default);
