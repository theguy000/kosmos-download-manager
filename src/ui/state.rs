use super::handlers::delete::{DeleteTarget, PendingDelete};
use super::projection::HistoryTracker;
use super::view::TableItem;
use crate::engine::{DownloadAction, DownloadSnapshot};
use crate::history::HistoryStore;
use crate::settings::SaveSettings;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tokio::sync::{mpsc, watch};

/// Description typed for the active download, tagged with the session it belongs to so a note
/// for an earlier download is never attached to a later one.
#[derive(Debug, Default)]
pub(super) struct ActiveDescription {
    session_id: u64,
    text: String,
}

impl ActiveDescription {
    pub(super) fn set(&mut self, session_id: u64, text: &str) {
        self.session_id = session_id;
        self.text = text.to_string();
    }

    /// The description typed for `session_id`, or empty when it was typed for another session.
    pub(super) fn get(&self, session_id: u64) -> &str {
        if self.session_id == session_id {
            &self.text
        } else {
            ""
        }
    }

    /// Like [`Self::get`], and forgets the note.
    pub(super) fn take(&mut self, session_id: u64) -> String {
        let text = self.get(session_id).to_string();
        *self = Self::default();
        text
    }
}

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) save_settings: Rc<RefCell<SaveSettings>>,
    pub(super) history_store: Rc<RefCell<HistoryStore>>,
    pub(super) history_tracker: Rc<RefCell<HistoryTracker>>,
    pub(super) download_history: Rc<slint::VecModel<TableItem>>,
    pub(super) displayed_delete_target: Rc<RefCell<Option<DeleteTarget>>>,
    pub(super) pending_delete: Rc<RefCell<Option<PendingDelete>>>,
    pub(super) active_description: Rc<RefCell<ActiveDescription>>,
    /// Session of the active download while its File Properties dialog is open.
    pub(super) properties_session: Rc<Cell<u64>>,
    pub(super) action_tx: mpsc::Sender<DownloadAction>,
    pub(super) snapshot_rx: watch::Receiver<DownloadSnapshot>,
}

impl AppState {
    pub(super) fn new(
        action_tx: mpsc::Sender<DownloadAction>,
        snapshot_rx: watch::Receiver<DownloadSnapshot>,
    ) -> Self {
        Self::from_parts(
            SaveSettings::load(),
            HistoryStore::load(),
            action_tx,
            snapshot_rx,
        )
    }

    /// Builds the state around already loaded settings and history, so tests can keep off the
    /// user's real files.
    pub(super) fn from_parts(
        save_settings: SaveSettings,
        history_store: HistoryStore,
        action_tx: mpsc::Sender<DownloadAction>,
        snapshot_rx: watch::Receiver<DownloadSnapshot>,
    ) -> Self {
        let next_id = history_store.next_id();
        Self {
            save_settings: Rc::new(RefCell::new(save_settings)),
            history_store: Rc::new(RefCell::new(history_store)),
            history_tracker: Rc::new(RefCell::new(HistoryTracker::new(next_id))),
            download_history: Rc::new(slint::VecModel::default()),
            displayed_delete_target: Rc::new(RefCell::new(None)),
            pending_delete: Rc::new(RefCell::new(None)),
            active_description: Rc::default(),
            properties_session: Rc::default(),
            action_tx,
            snapshot_rx,
        }
    }
}
