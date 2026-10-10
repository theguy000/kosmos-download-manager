use super::actions::send_action;
use super::table::remove_row_by_id;
use crate::engine::{DownloadAction, DownloadSnapshot, DownloadStatus};
use crate::history::{HistoryEntry, HistoryStore};
use crate::ui::projection::HistoryTracker;
use crate::ui::state::AppState;
use crate::ui::view::{MainWindow, TableItem};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteTarget {
    pub(crate) session_id: u64,
    pub(crate) status: DownloadStatus,
    pub(crate) completed_only: bool,
}

impl DeleteTarget {
    pub(crate) fn displayed(snapshot: &DownloadSnapshot) -> Option<Self> {
        (!matches!(snapshot.status, DownloadStatus::Idle)).then(|| Self {
            session_id: snapshot.session_id,
            status: snapshot.status.clone(),
            completed_only: false,
        })
    }

    pub(crate) fn action(&self, delete_file: bool) -> DownloadAction {
        DownloadAction::Remove {
            expected_session_id: self.session_id,
            expected_status: self.status.clone(),
            delete_file,
            completed_only: self.completed_only,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingDelete {
    Active(DeleteTarget),
    History {
        id: i32,
        filename: String,
        save_path: PathBuf,
    },
}

/// Shows the confirmation, or when the user turned it off, removes the download and keeps its files.
fn ask_or_remove(window: &MainWindow, ask: bool) {
    if ask {
        window.set_show_delete_dialog(true);
    } else {
        window.invoke_confirm_delete(false);
    }
}

/// Returns false when there is nothing to delete.
fn prepare_active_delete(
    window: &MainWindow,
    displayed: &RefCell<Option<DeleteTarget>>,
    pending: &RefCell<Option<PendingDelete>>,
    completed_only: bool,
) -> bool {
    let Some(mut request) = displayed.borrow().clone() else {
        return false;
    };
    if completed_only && !matches!(request.status, DownloadStatus::Completed) {
        return false;
    }

    let target_completed = matches!(request.status, DownloadStatus::Completed);
    request.completed_only = completed_only;
    *pending.borrow_mut() = Some(PendingDelete::Active(request));
    window.set_delete_filename(window.get_active_filename());
    window.set_delete_completed_only(completed_only);
    window.set_delete_target_completed(target_completed);
    window.set_delete_file(false);
    true
}

fn prepare_history_delete(
    window: &MainWindow,
    pending: &RefCell<Option<PendingDelete>>,
    entry: &HistoryEntry,
) {
    *pending.borrow_mut() = Some(PendingDelete::History {
        id: entry.id,
        filename: entry.filename.clone(),
        save_path: entry.save_path.clone(),
    });
    window.set_delete_filename(entry.filename.clone().into());
    window.set_delete_completed_only(false);
    window.set_delete_target_completed(true);
    window.set_delete_file(false);
}

pub(crate) fn clear_active_completed(
    window: &MainWindow,
    tracker: &RefCell<HistoryTracker>,
) -> Option<HistoryEntry> {
    let entry = tracker.borrow_mut().clear_completed();
    if entry.is_some() {
        window.set_completed_listed(false);
    }
    entry
}

/// Removes a listed download from the log, the table, and the selection.
pub(crate) fn complete_history_delete(
    window: &MainWindow,
    tracker: &RefCell<HistoryTracker>,
    store: &mut HistoryStore,
    history: &slint::VecModel<TableItem>,
    id: i32,
    failure: Option<String>,
) {
    if let Some(message) = failure {
        window.set_action_error_message(message.into());
    } else {
        if let Err(error) = store.remove(id) {
            window.set_action_error_message(
                format!("Could not save download history: {error}").into(),
            );
        }
        remove_row_by_id(history, id);
        if tracker
            .borrow()
            .completed()
            .as_ref()
            .is_some_and(|entry| entry.id == id)
        {
            clear_active_completed(window, tracker);
        }
        if window.get_selected_row() == id {
            window.set_selected_row(-1);
            window.set_history_row_selected(false);
        }
    }
    window.invoke_close_delete_dialog();
}

/// Deletes the file of a listed download off the UI thread, then reports back on the event loop.
fn spawn_history_file_delete(window: slint::Weak<MainWindow>, id: i32, save_path: PathBuf) {
    let display_path = save_path.display().to_string();
    tokio::spawn(async move {
        let failure =
            match tokio::task::spawn_blocking(move || std::fs::remove_file(save_path)).await {
                Ok(removal) => history_delete_failure(removal),
                Err(error) => Some(std::io::Error::other(error)),
            };
        let message = failure
            .map(|error| format!("Could not delete {display_path}: {error}"))
            .unwrap_or_default();
        let failed = !message.is_empty();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                window.invoke_history_delete_finished(id, failed, message.into());
            }
        });
    });
}

/// A file that is already gone counts as deleted; anything else is reported.
pub(crate) fn history_delete_failure(removal: std::io::Result<()>) -> Option<std::io::Error> {
    match removal {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(error),
    }
}

/// Empties the history: the log, the table, and everything that still points at a removed row.
/// Downloaded files are never touched. Tracker ids only grow, so new rows cannot reuse an id.
pub(crate) fn clear_history(window: &MainWindow, state: &AppState) {
    if let Err(error) = state.history_store.borrow_mut().clear() {
        window.set_action_error_message(format!("Could not save download history: {error}").into());
    }
    state.download_history.clear();
    clear_active_completed(window, &state.history_tracker);

    let pending_history_delete = matches!(
        *state.pending_delete.borrow(),
        Some(PendingDelete::History { .. })
    );
    if pending_history_delete {
        *state.pending_delete.borrow_mut() = None;
        window.set_show_delete_dialog(false);
        window.set_delete_file(false);
    }
    if window.get_selected_row() > 0 {
        window.set_selected_row(-1);
        window.set_history_row_selected(false);
    }
    if window.get_show_properties_dialog() && window.get_properties_row() > 0 {
        window.set_show_properties_dialog(false);
    }
}

pub(crate) fn bind_delete_handlers(window: &MainWindow, state: &AppState) {
    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_clear_history(move || {
            if let Some(window) = window_weak.upgrade() {
                clear_history(&window, &state);
            }
        });
    }

    {
        let displayed = state.displayed_delete_target.clone();
        let pending = state.pending_delete.clone();
        let store = state.history_store.clone();
        let settings = state.save_settings.clone();
        let window_weak = window.as_weak();
        window.on_request_delete_selected(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let ask = settings.borrow().ask_before_delete;
            let selected = window.get_selected_row();
            if selected == 0 {
                if prepare_active_delete(&window, &displayed, &pending, false) {
                    ask_or_remove(&window, ask);
                }
            } else {
                // Cloned: on_confirm_delete borrows the store mutably.
                let entry = store
                    .borrow()
                    .entries()
                    .iter()
                    .find(|e| e.id == selected)
                    .cloned();
                if let Some(entry) = entry {
                    prepare_history_delete(&window, &pending, &entry);
                    ask_or_remove(&window, ask);
                }
            }
        });
    }

    {
        let displayed = state.displayed_delete_target.clone();
        let pending = state.pending_delete.clone();
        let settings = state.save_settings.clone();
        let window_weak = window.as_weak();
        window.on_request_delete_completed(move || {
            if let Some(window) = window_weak.upgrade() {
                let ask = settings.borrow().ask_before_delete;
                if prepare_active_delete(&window, &displayed, &pending, true) {
                    ask_or_remove(&window, ask);
                }
            }
        });
    }

    {
        let pending = state.pending_delete.clone();
        window.on_dismiss_delete(move || {
            *pending.borrow_mut() = None;
        });
    }

    {
        let tx = state.action_tx.clone();
        let rx = state.snapshot_rx.clone();
        let pending = state.pending_delete.clone();
        let tracker = state.history_tracker.clone();
        let store = state.history_store.clone();
        let history = state.download_history.clone();
        let window_weak = window.as_weak();
        window.on_confirm_delete(move |delete_file| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Some(target) = pending.borrow_mut().take() else {
                return;
            };
            match target {
                PendingDelete::Active(mut request) => {
                    let snapshot = rx.borrow();
                    if snapshot.status == DownloadStatus::Idle
                        || snapshot.session_id != request.session_id
                    {
                        window.invoke_close_delete_dialog();
                        return;
                    }
                    request.status = snapshot.status.clone();

                    if let Some(entry) = clear_active_completed(&window, &tracker) {
                        let _ = store.borrow_mut().remove(entry.id);
                        remove_row_by_id(&history, entry.id);
                    }

                    if send_action(&window, &tx, request.action(delete_file)) {
                        window.set_active_error_message("".into());
                        window.invoke_close_delete_dialog();
                    }
                }
                PendingDelete::History { id, save_path, .. } => {
                    if delete_file {
                        spawn_history_file_delete(window.as_weak(), id, save_path);
                    } else {
                        complete_history_delete(
                            &window,
                            &tracker,
                            &mut store.borrow_mut(),
                            &history,
                            id,
                            None,
                        );
                    }
                }
            }
        });
    }

    {
        let tracker = state.history_tracker.clone();
        let store = state.history_store.clone();
        let history = state.download_history.clone();
        let window_weak = window.as_weak();
        window.on_history_delete_finished(move |id, failed, message| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            complete_history_delete(
                &window,
                &tracker,
                &mut store.borrow_mut(),
                &history,
                id,
                failed.then(|| message.to_string()),
            );
        });
    }
}
