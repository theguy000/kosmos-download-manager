use crate::engine::DownloadSnapshot;
use crate::history::HistoryEntry;
use crate::ui::projection::{active_properties, history_properties};
use crate::ui::state::AppState;
use crate::ui::view::MainWindow;
use slint::ComponentHandle;

pub(crate) fn bind_properties_handlers(window: &MainWindow, state: &AppState) {
    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_open_properties(move |id| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let settings = state.save_settings.borrow();
            // Row 0 is the active download; every other row is a listed history entry.
            let (properties, description) = if id == 0 {
                let snap = state.snapshot_rx.borrow();
                state.properties_session.set(snap.session_id);
                let description = state
                    .active_description
                    .borrow()
                    .get(snap.session_id)
                    .to_string();
                (active_properties(&window, &settings, &snap), description)
            } else {
                let store = state.history_store.borrow();
                let Some(entry) = store.get(id) else {
                    return;
                };
                (
                    history_properties(&settings, entry),
                    entry.description.clone(),
                )
            };
            window.set_properties(properties);
            window.set_properties_description(description.into());
            window.set_properties_row(id);
            window.set_show_properties_dialog(true);
        });
    }

    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_save_description(move |id, description| {
            let description = description.trim();
            if id == 0 {
                // The session the dialog opened for, not whichever one is active by now.
                let session = state.properties_session.get();
                state
                    .active_description
                    .borrow_mut()
                    .set(session, description);
                return;
            }
            let saved = state
                .history_store
                .borrow_mut()
                .set_description(id, description);
            if let (Err(error), Some(window)) = (saved, window_weak.upgrade()) {
                window.set_action_error_message(
                    format!("Could not save description: {error}").into(),
                );
            }
        });
    }
}

/// Keeps an open dialog for the active download pointed at it. When the download is listed
/// (`listed`), the dialog moves to its history row so the description still lands on it; when
/// another session has taken over, the dialog closes instead of editing the wrong download.
pub(crate) fn follow_active_download(
    window: &MainWindow,
    state: &AppState,
    snap: &DownloadSnapshot,
    listed: Option<&HistoryEntry>,
) {
    if !window.get_show_properties_dialog() || window.get_properties_row() != 0 {
        return;
    }
    if snap.session_id != state.properties_session.get() {
        window.set_show_properties_dialog(false);
    } else if let Some(entry) = listed {
        window.set_properties_row(entry.id);
    }
}
