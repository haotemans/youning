//! Bounded in-memory counters shared by tool and skill calls, retained on reload.
use serde::Serialize;
use std::{
    sync::{Arc, RwLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default, Serialize)]
pub(super) struct Snapshot {
    pub active: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub rejected: u64,
    pub last_duration_ms: Option<u64>,
    pub last_finished_at: Option<u64>,
    pub last_status: Option<&'static str>,
    pub last_error: Option<String>,
}

#[derive(Default)]
pub(super) struct Invocations(RwLock<Snapshot>);
impl Invocations {
    pub fn snapshot(&self) -> Snapshot {
        self.0.read().unwrap().clone()
    }
    pub fn reject(&self) {
        self.0.write().unwrap().rejected += 1;
    }
    pub fn begin(self: &Arc<Self>) -> Invocation {
        self.0.write().unwrap().active += 1;
        Invocation {
            stats: self.clone(),
            started: Instant::now(),
            finished: false,
        }
    }
}

pub(super) struct Invocation {
    stats: Arc<Invocations>,
    started: Instant,
    finished: bool,
}
impl Invocation {
    pub fn finish(mut self, error: Option<String>) {
        let mut stats = self.stats.0.write().unwrap();
        stats.active -= 1;
        if error.is_some() {
            stats.failed += 1;
            stats.last_status = Some("failed");
        } else {
            stats.succeeded += 1;
            stats.last_status = Some("succeeded");
        }
        stats.last_duration_ms = Some(self.started.elapsed().as_millis() as u64);
        stats.last_finished_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|v| v.as_secs());
        stats.last_error = error;
        self.finished = true;
    }
}
impl Drop for Invocation {
    fn drop(&mut self) {
        if !self.finished {
            let mut stats = self.stats.0.write().unwrap();
            stats.active -= 1;
            stats.cancelled += 1;
        }
    }
}
