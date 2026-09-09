//! Bounded, host-owned disk indexing for browser LSP sessions.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use openmat_lsp::{Server, WorkspaceDocument, WorkspaceSnapshot};

use crate::workspace::WorkspaceService;

const CONTEXT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);
const WATCH_FAILURE_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) struct WorkspaceIndex {
    service: WorkspaceService,
    watcher: Option<RecommendedWatcher>,
    changed: Arc<AtomicBool>,
    watch_active: bool,
    target: Option<(u64, PathBuf)>,
    search_paths: Vec<String>,
    documents: BTreeMap<String, String>,
    incomplete_reason: Option<String>,
    reported_complete: Option<bool>,
    checked_at: Option<Instant>,
    scanned_at: Option<Instant>,
    pending_since: Option<Instant>,
}

impl WorkspaceIndex {
    pub(crate) fn new(service: WorkspaceService) -> Self {
        // Coalesce notifications into a single flag instead of retaining an
        // unbounded queue when a build or a large copy touches many files.
        let changed = Arc::new(AtomicBool::new(false));
        let watch_changed = Arc::clone(&changed);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if !matches!(
                event,
                Ok(notify::Event {
                    kind: EventKind::Access(_),
                    ..
                })
            ) {
                watch_changed.store(true, Ordering::Release);
            }
        })
        .ok();
        Self {
            service,
            watcher,
            changed,
            watch_active: false,
            target: None,
            search_paths: Vec::new(),
            documents: BTreeMap::new(),
            incomplete_reason: None,
            reported_complete: None,
            checked_at: None,
            scanned_at: None,
            pending_since: None,
        }
    }

    #[allow(clippy::too_many_lines)] // One synchronization step keeps root, source and completeness transitions atomic.
    pub(crate) fn refresh(&mut self, server: &mut Server, force_scan: bool) -> Option<String> {
        let now = Instant::now();
        if !force_scan
            && self
                .checked_at
                .is_some_and(|checked| now.duration_since(checked) < CONTEXT_POLL_INTERVAL)
        {
            return None;
        }
        self.checked_at = Some(now);
        let target = match self.service.current_directory_watch_target() {
            Ok(target) => target,
            Err(error) => {
                self.documents.clear();
                self.incomplete_reason = Some(error.to_string());
                return self.publish(server);
            }
        };
        let root_changed = self.target.as_ref() != Some(&target);
        if root_changed {
            if self.watch_active
                && let Some((_, old_path)) = &self.target
                && let Some(watcher) = &mut self.watcher
            {
                let _ = watcher.unwatch(old_path);
            }
            self.watch_active = self
                .watcher
                .as_mut()
                .is_some_and(|watcher| watcher.watch(&target.1, RecursiveMode::Recursive).is_ok());
            self.target = Some(target.clone());
            self.documents.clear();
            self.pending_since = None;
        }
        if self.changed.swap(false, Ordering::AcqRel) {
            self.pending_since.get_or_insert(now);
        }
        let search_paths = self.service.search_path().snapshot().map_or_else(
            |_| Vec::new(),
            |snapshot| {
                snapshot
                    .paths
                    .iter()
                    .filter_map(|path| relative_path(&target.1, path))
                    .collect()
            },
        );
        let context_changed = root_changed || search_paths != self.search_paths;
        self.search_paths = search_paths;
        let scan_due = force_scan
            || root_changed
            || self
                .pending_since
                .is_some_and(|pending| now.duration_since(pending) >= CHANGE_DEBOUNCE)
            || (!self.watch_active
                && self.scanned_at.is_none_or(|scanned| {
                    now.duration_since(scanned) >= WATCH_FAILURE_POLL_INTERVAL
                }));
        let mut sources_changed = false;
        if scan_due {
            self.pending_since = None;
            self.scanned_at = Some(now);
            match self.service.lsp_source_snapshot(target.0) {
                Ok(snapshot) => {
                    sources_changed = self.documents != snapshot.documents
                        || self.incomplete_reason != snapshot.incomplete_reason;
                    self.documents = snapshot.documents;
                    self.incomplete_reason = snapshot.incomplete_reason;
                }
                Err(error) => {
                    self.documents.clear();
                    self.incomplete_reason = Some(error.to_string());
                    sources_changed = true;
                }
            }
        }
        // Never publish a just-scanned old root under a new kernel context.
        if self.service.current_directory_watch_target().ok().as_ref() != Some(&target) {
            self.checked_at = None;
            self.documents.clear();
            self.incomplete_reason = Some("Current Folder changed during indexing.".into());
            return self.publish(server);
        }
        if context_changed || sources_changed {
            return self.publish(server);
        }
        None
    }

    fn publish(&mut self, server: &mut Server) -> Option<String> {
        let target = self.target.as_ref()?;
        let root_text = target.1.to_str()?;
        let root_uri = workspace_root_uri(target.0, root_text);
        let documents = self
            .documents
            .iter()
            .map(|(path, text)| WorkspaceDocument {
                uri: format!("{root_uri}{}", encoded_relative_path(path)),
                relative_path: path.clone(),
                text: text.clone(),
            })
            .collect();
        // Files are bounded well below the LSP document limit and are UTF-8
        // checked by WorkspaceService; replacement is atomic in the core.
        if let Err(error) = server.replace_workspace_documents(WorkspaceSnapshot {
            root_uri,
            current_directory: String::new(),
            search_paths: self.search_paths.clone(),
            documents,
            complete: self.incomplete_reason.is_none(),
        }) {
            eprintln!("openmat-server: workspace language index update failed: {error}");
        }
        let complete = self.incomplete_reason.is_none();
        let previous = self.reported_complete.replace(complete);
        if previous != Some(complete) {
            if let Some(reason) = &self.incomplete_reason {
                return Some(format!(
                    "Workspace language index is incomplete; rename is temporarily unavailable. {reason}"
                ));
            }
            if previous == Some(false) {
                return Some(
                    "Workspace language index is complete; rename is available again.".into(),
                );
            }
        }
        None
    }
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()?
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|components| components.join("/"))
}

pub(crate) fn workspace_root_uri(generation: u64, root: &str) -> String {
    // Match workspaceDocumentUri followed by Monaco Uri.toString(): JavaScript
    // encodeURIComponent for the inner root, RFC3986 for the canonical outer URI.
    format!(
        "openmat-workspace://root-{generation}/{}/",
        percent_encode(&percent_encode(root, true), false)
    )
}

pub(crate) fn encoded_relative_path(path: &str) -> String {
    path.split('/')
        .map(|segment| percent_encode(segment, false))
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode(text: &str, javascript: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (javascript && matches!(byte, b'!' | b'\'' | b'(' | b')' | b'*'))
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_uris_match_monaco_canonical_encoding() {
        assert_eq!(
            workspace_root_uri(7, "E:\\项目 空格\\100%"),
            "openmat-workspace://root-7/E%253A%255C%25E9%25A1%25B9%25E7%259B%25AE%2520%25E7%25A9%25BA%25E6%25A0%25BC%255C100%2525/"
        );
        assert_eq!(
            workspace_root_uri(1, "/root/目录/%20/!'()*"),
            "openmat-workspace://root-1/%252Froot%252F%25E7%259B%25AE%25E5%25BD%2595%252F%252520%252F%21%27%28%29%2A/"
        );
        assert_eq!(
            encoded_relative_path("+包/it's (ready)!.m"),
            "%2B%E5%8C%85/it%27s%20%28ready%29%21.m"
        );
    }
}
