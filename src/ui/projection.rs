use super::format::{format_bytes, format_eta, format_speed};
use super::view::{ChunkVisual, FileProperties, MainWindow, Navigation, Palette};
use crate::engine::{ChunkSnapshot, DownloadSnapshot, DownloadStatus};
use crate::history::{HistoryEntry, downloaded_label, now_unix_ms};
use crate::settings::SaveSettings;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::path::Path;
use tokio::sync::watch;

pub(super) fn should_project_snapshot<T>(snapshot: &watch::Ref<'_, T>, initial: &mut bool) -> bool {
    // Unlike Receiver::has_changed, this reports a final unseen value after closure.
    let should_project = *initial || snapshot.has_changed();
    *initial = false;
    should_project
}

pub(super) fn category_matches_id(
    nav: &Navigation,
    filter_id: i32,
    category_id: i32,
    completed: bool,
) -> bool {
    if filter_id == nav.get_all() {
        return true;
    }
    if (nav.get_first_category()..=nav.get_last_category()).contains(&filter_id) {
        return filter_id == category_id;
    }
    if filter_id == nav.get_unfinished() {
        return !completed;
    }
    if filter_id == nav.get_grabber() || filter_id == nav.get_queues() {
        return false;
    }
    filter_id == nav.get_finished() && completed
}

pub(super) fn history_table_item(
    settings: &SaveSettings,
    entry: &HistoryEntry,
) -> super::view::TableItem {
    let file_type = settings.category_for_filename(&entry.filename);
    super::view::TableItem {
        id: entry.id,
        filename: entry.filename.clone().into(),
        file_type: file_type.as_str().into(),
        category_id: file_type.category_id(),
        size_text: format_bytes(entry.total_bytes).into(),
        status_text: "Complete".into(),
        time_left_text: "--:--".into(),
        transfer_rate_text: "0 KB/s".into(),
        downloaded_text: downloaded_label(entry.completed_unix_ms).into(),
        size_bytes: entry.total_bytes as f32,
    }
}

/// The fields every download shares; the caller fills in status, size and result.
fn base_properties(
    settings: &SaveSettings,
    filename: &str,
    save_path: &Path,
    url: &str,
) -> FileProperties {
    let file_type = settings.category_for_filename(filename);
    FileProperties {
        filename: filename.into(),
        file_type: file_type.as_str().into(),
        type_text: file_type.display_name().into(),
        save_to: save_path.to_string_lossy().as_ref().into(),
        address: url.into(),
        ..Default::default()
    }
}

pub(super) fn history_properties(settings: &SaveSettings, entry: &HistoryEntry) -> FileProperties {
    FileProperties {
        status_text: "Complete".into(),
        size_text: format_bytes(entry.total_bytes).into(),
        ..base_properties(settings, &entry.filename, &entry.save_path, &entry.url)
    }
}

/// Properties of the active download. Status and size reuse the text the row shows; the result
/// comes from the snapshot, so it outlives the "Download failed" dialog.
pub(super) fn active_properties(
    window: &MainWindow,
    settings: &SaveSettings,
    snap: &DownloadSnapshot,
) -> FileProperties {
    let filename = if snap.filename.is_empty() {
        "New Download"
    } else {
        &snap.filename
    };
    FileProperties {
        status_text: window.get_active_status(),
        size_text: window.get_active_size(),
        result_text: match &snap.status {
            DownloadStatus::Failed(message) => message.as_str().into(),
            _ => slint::SharedString::default(),
        },
        ..base_properties(settings, filename, &snap.save_path, &snap.url)
    }
}
/// Reorders the listed rows for a header sort. Columns 0, 1 and 3 are sortable (File Name,
/// Size, Date Added); any other column leaves the order untouched. Ties keep a stable order
/// by row id.
pub(super) fn sort_items(items: &mut [super::view::TableItem], column: i32, ascending: bool) {
    use std::cmp::Ordering;

    match column {
        0 => items.sort_by_cached_key(|a| (a.filename.to_lowercase(), a.id)),
        1 => items.sort_by(|a, b| {
            a.size_bytes
                .partial_cmp(&b.size_bytes)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        }),
        // ponytail: history IDs strictly increment with completion order; sorting by id avoids
        // adding a timestamp field to TableItem.
        3 => items.sort_by_key(|a| a.id),
        _ => return,
    }

    if !ascending {
        items.reverse();
    }
}

pub(super) fn update_window_state(
    window: &MainWindow,
    settings: &SaveSettings,
    snap: &DownloadSnapshot,
) {
    let (is_downloading, is_paused, is_completed) = match &snap.status {
        DownloadStatus::Connecting | DownloadStatus::Downloading => (true, false, false),
        DownloadStatus::Paused => (false, true, false),
        DownloadStatus::Completed => (false, false, true),
        DownloadStatus::Failed(_) => (false, snap.resumable, false),
        DownloadStatus::Idle => (false, false, false),
    };

    window.set_has_active_download(!matches!(snap.status, DownloadStatus::Idle));
    window.set_is_downloading(is_downloading);
    window.set_is_paused(is_paused);
    window.set_is_completed(is_completed);
    window.set_is_resumable(snap.resumable);

    window.set_active_filename(snap.filename.clone().into());
    let file_type = settings.category_for_filename(&snap.filename);
    window.set_active_file_type(file_type.as_str().into());
    window.set_active_category_id(file_type.category_id());

    let (idle_color, warning_color, success_color, danger_color) = {
        let palette = window.global::<Palette>();
        (
            palette.get_text_faint(),
            palette.get_warning(),
            palette.get_success(),
            palette.get_danger_text(),
        )
    };

    let (badge, color, err) = match &snap.status {
        DownloadStatus::Idle => ("Idle", idle_color, String::new()),
        DownloadStatus::Connecting => ("Connecting", warning_color, String::new()),
        DownloadStatus::Downloading => ("Downloading", success_color, String::new()),
        DownloadStatus::Paused => ("Stopped", warning_color, String::new()),
        DownloadStatus::Completed => ("Complete", success_color, String::new()),
        DownloadStatus::Failed(msg) => ("Failed", danger_color, msg.clone()),
    };

    let progress = match snap.total_bytes {
        Some(total) if total > 0 => (snap.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0),
        _ if is_completed => 1.0,
        _ => 0.0,
    };

    let status_str = if is_downloading {
        format!("{badge} ({:.1}%)", progress * 100.0)
    } else {
        badge.to_string()
    };
    window.set_active_status(status_str.into());
    window.set_active_status_color(color);
    window.set_active_error_message(err.into());

    window.set_active_transfer_rate(format_speed(snap.speed_bytes_per_sec).into());
    let size_str = match snap.total_bytes {
        Some(total) if !is_completed => format!(
            "{} / {}",
            format_bytes(snap.downloaded_bytes),
            format_bytes(total)
        ),
        Some(total) => format_bytes(total),
        None => format_bytes(snap.downloaded_bytes),
    };
    window.set_active_size(size_str.into());
    window.set_active_time_left(format_eta(snap.eta_seconds).into());

    project_chunks(window, &snap.chunks);

    if let Some(ref prompt) = snap.duplicate {
        if !window.get_show_duplicate_dialog() {
            window.set_duplicate_selected_option(0);
            window.set_duplicate_remember(false);
            window.set_show_add_dialog(false);
            window.set_show_duplicate_dialog(true);
        }
        window.set_duplicate_url(prompt.url.clone().into());
        window.set_duplicate_filename(prompt.filename.clone().into());
        window.set_duplicate_is_link(prompt.link_duplicate);
        let size_str = prompt.existing_bytes.map(format_bytes).unwrap_or_default();
        window.set_duplicate_existing_size(size_str.into());
    } else if window.get_show_duplicate_dialog() {
        window.set_show_duplicate_dialog(false);
    }
}

/// Follows the active session and reports the download it finishes.
pub(crate) struct HistoryTracker {
    /// Id for the next listed download. Never 0, which marks the active download.
    next_item_id: i32,
    session_id: u64,
    completed: Option<HistoryEntry>,
}

impl HistoryTracker {
    pub(crate) fn new(loaded_next_id: i32) -> Self {
        Self {
            next_item_id: loaded_next_id,
            session_id: 0,
            completed: None,
        }
    }

    pub(crate) fn completed(&self) -> Option<&HistoryEntry> {
        self.completed.as_ref()
    }

    /// Records `snap` and returns the finished download.
    ///
    /// The first snapshot a session reports `Completed` yields its entry, so it can be persisted
    /// and listed right away. A later session clears the kept entry without re-listing it.
    pub(crate) fn observe(&mut self, snap: &DownloadSnapshot) -> Option<HistoryEntry> {
        if snap.session_id != self.session_id {
            self.session_id = snap.session_id;
            self.completed = None;
        }

        match snap.status {
            // The first snapshot to report the finish is the one kept, so the listed row keeps
            // the id and the completion time of that first report.
            DownloadStatus::Completed if self.completed.is_none() => {
                let entry = finished_download(self.next_item_id, snap);
                self.next_item_id = self.next_item_id.saturating_add(1);
                self.completed = Some(entry.clone());
                Some(entry)
            }
            DownloadStatus::Idle => {
                self.completed = None;
                None
            }
            _ => None,
        }
    }

    /// Drops the pending download, matching a duplicate answer that replaces the file.
    pub(crate) fn clear_completed(&mut self) -> Option<HistoryEntry> {
        self.completed.take()
    }
}

fn finished_download(id: i32, snap: &DownloadSnapshot) -> HistoryEntry {
    HistoryEntry {
        id,
        url: snap.url.clone(),
        filename: snap.filename.clone(),
        save_path: snap.save_path.clone(),
        total_bytes: snap.total_bytes.unwrap_or(snap.downloaded_bytes),
        completed_unix_ms: now_unix_ms(),
        description: String::new(),
    }
}

pub(super) fn chunk_progress(chunk: &ChunkSnapshot) -> f32 {
    if chunk.is_done {
        1.0
    } else if chunk.total > 0 {
        (chunk.downloaded as f32 / chunk.total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Updates the chunk model in place so Slint only redraws rows that changed.
fn project_chunks(window: &MainWindow, chunks: &[ChunkSnapshot]) {
    let visuals = chunks.iter().map(|c| ChunkVisual {
        id: i32::try_from(c.id).unwrap_or(i32::MAX),
        progress: chunk_progress(c),
        is_done: c.is_done,
    });
    let model = window.get_active_chunks();
    let Some(vec_model) = model.as_any().downcast_ref::<VecModel<ChunkVisual>>() else {
        window.set_active_chunks(ModelRc::new(VecModel::from_iter(visuals)));
        return;
    };
    if vec_model.row_count() != chunks.len() {
        vec_model.set_vec(visuals.collect::<Vec<_>>());
        return;
    }
    for (row, visual) in visuals.enumerate() {
        if vec_model.row_data(row).as_ref() != Some(&visual) {
            vec_model.set_row_data(row, visual);
        }
    }
}
