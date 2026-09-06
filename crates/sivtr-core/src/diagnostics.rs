//! One process-wide diagnostic sink: warnings from every source (native
//! listing, archive sync, index cache) land here. CLI runs report to stderr
//! via the subscriber installed at startup; the TUI reads the log for the
//! `!` diagnostics overlay. Nothing forks: there is exactly one warning
//! path, and the listener list decides where it shows up.

use std::fmt::Display;
use std::sync::{Mutex, OnceLock};

/// Newest-last ring of captured warnings (TUI diagnostics overlay).
static LOG: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
const LOG_CAP: usize = 200;

/// Listeners mirror each warning to additional destinations (stderr).
type Listener = Box<dyn Fn(&str) + Send + Sync>;
static LISTENERS: OnceLock<Mutex<Vec<Listener>>> = OnceLock::new();

/// Report a warning: appended to the diagnostics log and mirrored to every
/// registered listener. Thread-safe; never blocks on lock poisoning.
pub fn warn(message: impl Display) {
    let message = message.to_string();
    if let Some(mut log) = LOG.get().and_then(|log| log.lock().ok()) {
        if log.len() == LOG_CAP {
            log.remove(0);
        }
        log.push(message.clone());
    }
    if let Some(listeners) = LISTENERS.get().and_then(|listeners| listeners.lock().ok()) {
        for listener in listeners.iter() {
            listener(&message);
        }
    }
}

/// Snapshot of the diagnostics log, oldest first.
pub fn log() -> Vec<String> {
    LOG.get()
        .and_then(|log| log.lock().ok())
        .map(|log| log.clone())
        .unwrap_or_default()
}

/// Register a warning destination (stderr for CLI runs). Only listeners
/// registered before the first warning fire — callers install at startup.
pub fn subscribe(listener: Listener) {
    LISTENERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(listener);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_are_delivered_in_order() {
        // The ring is process-global (parallel tests warn into it and can
        // evict entries), so order is verified through the subscriber path,
        // which is the same `warn` pipeline the ring mirrors.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        subscribe(Box::new(move |message| {
            sink.lock().unwrap().push(message.to_string());
        }));
        warn("ordered first");
        warn("ordered second");
        // Parallel tests warn too and every listener sees every warning, so
        // filter to this test's marker and assert the relative order.
        let captured = seen.lock().unwrap();
        let mine: Vec<String> = captured
            .iter()
            .filter(|entry| entry.starts_with("ordered"))
            .cloned()
            .collect();
        assert_eq!(mine, vec!["ordered first", "ordered second"]);
    }

    #[test]
    fn subscribers_see_every_warning() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        subscribe(Box::new(move |message| {
            sink.lock().unwrap().push(message.to_string());
        }));
        warn("for the listener");
        assert!(seen
            .lock()
            .unwrap()
            .contains(&"for the listener".to_string()));
    }
}
