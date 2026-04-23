//! Filesystem watcher: observe vault changes made outside the app.
//!
//! Wraps `notify-debouncer-full` so the rest of the codebase sees a
//! clean set of [`VaultChange`] events instead of raw OS filesystem
//! events. Debounces short bursts (editor saves often fire a create +
//! modify + close in quick succession) into a single event per path.
//!
//! The watcher is cross-platform via `notify`'s auto-selection of the
//! platform backend (ReadDirectoryChangesW / inotify / FSEvents / kqueue).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};

/// A vault change worth reacting to. Path is absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultChange {
    Added(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

/// A live watcher. Dropping it stops the background thread.
pub struct VaultWatcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    pub events: Receiver<Vec<VaultChange>>,
}

impl VaultWatcher {
    /// Start watching a vault root recursively. Changes are batched
    /// every ~150ms to smooth out editor save bursts.
    pub fn start(root: impl AsRef<Path>) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut debouncer = new_debouncer(
            Duration::from_millis(150),
            None,
            move |result: DebounceEventResult| {
                let events = match result {
                    Ok(events) => events,
                    Err(_) => return,
                };
                let translated = translate_events(events);
                if !translated.is_empty() {
                    let _ = tx.send(translated);
                }
            },
        )?;

        debouncer
            .watch(root.as_ref(), RecursiveMode::Recursive)?;

        Ok(Self {
            _debouncer: debouncer,
            events: rx,
        })
    }
}

fn translate_events(
    events: Vec<notify_debouncer_full::DebouncedEvent>,
) -> Vec<VaultChange> {
    let mut out = Vec::new();
    for ev in events {
        if should_ignore_path(&ev.event.paths) {
            continue;
        }
        match ev.event.kind {
            EventKind::Create(_) => {
                for p in ev.event.paths {
                    out.push(VaultChange::Added(p));
                }
            }
            EventKind::Modify(kind) => {
                // Most `Modify` events are content changes. Rename is
                // reported as `Modify(Name)` with From+To paths.
                if matches!(kind, notify::event::ModifyKind::Name(_)) && ev.event.paths.len() >= 2 {
                    out.push(VaultChange::Renamed {
                        from: ev.event.paths[0].clone(),
                        to: ev.event.paths[1].clone(),
                    });
                } else {
                    for p in ev.event.paths {
                        out.push(VaultChange::Modified(p));
                    }
                }
            }
            EventKind::Remove(_) => {
                for p in ev.event.paths {
                    out.push(VaultChange::Removed(p));
                }
            }
            _ => {}
        }
    }
    out
}

/// Skip hidden dirs and `.memex/` internals. We care about notes /
/// journal / attachments; anything else is noise.
fn should_ignore_path(paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| {
        p.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s.starts_with('.')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The watcher talks to the OS, so we can't fully unit-test it in
    // CI without timing flakiness. These tests cover the pure parts.

    #[test]
    fn ignore_hidden_paths() {
        assert!(should_ignore_path(&[PathBuf::from("/vault/.memex/cache")]));
        assert!(should_ignore_path(&[PathBuf::from("/vault/.git/HEAD")]));
        assert!(!should_ignore_path(&[PathBuf::from("/vault/notes/foo.md")]));
    }
}
