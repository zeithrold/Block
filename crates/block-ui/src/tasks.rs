//! Real downloader observations. Overall totals stay unknown until the engine
//! supplies a plan; observed files are never presented as the complete plan.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Task observations are internal to native UI and engine."
)]

use block_launcher::DownloadEvent;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(super) struct Transfer {
    pub id: String,
    pub size: Option<u64>,
    pub received: u64,
    pub complete: bool,
    pub cached: bool,
    pub retries: u32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct DownloadTracker {
    pub revision: u64,
    pub transport_bytes: u64,
    files: BTreeMap<String, Transfer>,
}

impl DownloadTracker {
    pub(super) fn apply(&mut self, event: DownloadEvent) {
        self.revision = self.revision.wrapping_add(1);
        match event {
            DownloadEvent::Started { sha1, size } => {
                self.files.insert(
                    sha1.clone(),
                    Transfer {
                        id: sha1,
                        size: Some(size),
                        ..Default::default()
                    },
                );
            }
            DownloadEvent::Bytes { sha1, bytes } => {
                let transfer = self.files.entry(sha1.clone()).or_insert_with(|| Transfer {
                    id: sha1,
                    ..Default::default()
                });
                transfer.received = transfer.received.saturating_add(bytes as u64);
                self.transport_bytes = self.transport_bytes.saturating_add(bytes as u64);
            }
            DownloadEvent::Complete { sha1, cached } => {
                let transfer = self.files.entry(sha1.clone()).or_insert_with(|| Transfer {
                    id: sha1,
                    ..Default::default()
                });
                transfer.complete = true;
                transfer.cached = cached;
                if let Some(size) = transfer.size {
                    transfer.received = size;
                }
            }
            DownloadEvent::Retry { sha1, attempt, .. } => {
                if let Some(transfer) = self.files.get_mut(&sha1) {
                    transfer.received = 0;
                    transfer.retries = attempt;
                }
            }
        }
    }

    pub(super) fn verified(&self) -> usize {
        self.files.values().filter(|file| file.complete).count()
    }
    pub(super) fn transfers(&self) -> impl Iterator<Item = &Transfer> {
        self.files.values()
    }
}

#[cfg(test)]
mod tests {
    use super::DownloadTracker;
    use block_launcher::DownloadEvent;
    #[test]
    fn retries_restart_file_progress_without_erasing_actual_transport_bytes() {
        let mut tracker = DownloadTracker::default();
        tracker.apply(DownloadEvent::Started {
            sha1: "object".into(),
            size: 100,
        });
        tracker.apply(DownloadEvent::Bytes {
            sha1: "object".into(),
            bytes: 75,
        });
        tracker.apply(DownloadEvent::Retry {
            sha1: "object".into(),
            url: "https://example.invalid/object".into(),
            attempt: 1,
        });
        assert_eq!(
            tracker.transfers().next().map(|file| file.received),
            Some(0)
        );
        assert_eq!(tracker.transport_bytes, 75);
        tracker.apply(DownloadEvent::Complete {
            sha1: "object".into(),
            cached: false,
        });
        tracker.apply(DownloadEvent::Complete {
            sha1: "cached".into(),
            cached: true,
        });
        assert_eq!(tracker.verified(), 2);
        assert_eq!(
            tracker
                .transfers()
                .find(|file| file.cached)
                .and_then(|file| file.size),
            None
        );
    }
}
