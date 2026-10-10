use super::support::{finished_entry, install_test_platform};
use crate::engine::{DownloadAction, DownloadSnapshot, DownloadStatus};
use crate::history::tests::TempFile;
use crate::settings::SaveSettings;
use crate::ui::handlers::delete::{DeleteTarget, bind_delete_handlers, history_delete_failure};
use crate::ui::state::AppState;
use crate::ui::view::MainWindow;
use std::io::{Error, ErrorKind};
use tokio::sync::{mpsc, watch};

#[test]
fn delete_target_preserves_exact_confirmation_token() {
    let completed = DeleteTarget {
        session_id: 41,
        status: DownloadStatus::Completed,
        completed_only: true,
    };
    match completed.action(true) {
        DownloadAction::Remove {
            expected_session_id,
            expected_status,
            delete_file,
            completed_only,
        } => {
            assert_eq!(expected_session_id, 41);
            assert_eq!(expected_status, DownloadStatus::Completed);
            assert!(delete_file);
            assert!(completed_only);
        }
        _ => panic!("delete target must produce Remove"),
    }
}

#[test]
fn a_missing_file_counts_as_deleted() {
    assert!(history_delete_failure(Ok(())).is_none());
    assert!(
        history_delete_failure(Err(Error::from(ErrorKind::NotFound))).is_none(),
        "a file that is already gone is what the user asked for"
    );
    assert_eq!(
        history_delete_failure(Err(Error::from(ErrorKind::PermissionDenied)))
            .map(|error| error.kind()),
        Some(ErrorKind::PermissionDenied),
        "every other failure is reported"
    );
}

type Bound = (
    MainWindow,
    AppState,
    mpsc::Receiver<DownloadAction>,
    watch::Sender<DownloadSnapshot>,
);

/// Delete handlers bound to a window over a temporary history file and chosen settings.
fn bound_window(
    confirm_delete: bool,
    history: &TempFile,
) -> Result<Bound, Box<dyn std::error::Error>> {
    let (_window, _clipboard) = install_test_platform()?;
    let ui = MainWindow::new()?;
    let (action_tx, action_rx) = mpsc::channel(1);
    let (snapshot_tx, snapshot_rx) = watch::channel(DownloadSnapshot::default());
    let settings = SaveSettings {
        confirm_delete,
        ..Default::default()
    };
    let state = AppState::from_parts(settings, history.store(), action_tx, snapshot_rx);
    bind_delete_handlers(&ui, &state);
    Ok((ui, state, action_rx, snapshot_tx))
}

#[test]
fn listed_download_is_removed_without_asking_and_its_file_stays()
-> Result<(), Box<dyn std::error::Error>> {
    let history = TempFile::new("delete-off");
    let file = TempFile::new("delete-off-file");
    std::fs::write(file.path(), b"data")?;
    let mut entry = finished_entry(1, "a.zip", 4);
    entry.save_path = file.path().to_path_buf();
    history.store().record(entry)?;
    let (ui, _state, _actions, _snapshots) = bound_window(false, &history)?;

    ui.set_selected_row(1);
    ui.invoke_request_delete_selected();

    assert!(!ui.get_show_delete_dialog());
    assert!(history.store().get(1).is_none(), "removed from the list");
    assert!(file.path().exists(), "the file is never deleted unasked");
    Ok(())
}

#[test]
fn listed_download_still_asks_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let history = TempFile::new("delete-on");
    history.store().record(finished_entry(1, "a.zip", 4))?;
    let (ui, _state, _actions, _snapshots) = bound_window(true, &history)?;

    ui.set_selected_row(1);
    ui.invoke_request_delete_selected();

    assert!(ui.get_show_delete_dialog());
    assert!(history.store().get(1).is_some(), "nothing is removed yet");
    Ok(())
}

#[test]
fn active_download_is_removed_without_asking_and_keeps_the_partial_file()
-> Result<(), Box<dyn std::error::Error>> {
    let history = TempFile::new("delete-active-off");
    let (ui, state, mut actions, snapshots) = bound_window(false, &history)?;
    let snapshot = DownloadSnapshot {
        session_id: 3,
        status: DownloadStatus::Downloading,
        ..Default::default()
    };
    snapshots.send(snapshot.clone())?;
    *state.displayed_delete_target.borrow_mut() = DeleteTarget::displayed(&snapshot);

    ui.set_selected_row(0);
    ui.invoke_request_delete_selected();

    assert!(!ui.get_show_delete_dialog());
    assert!(matches!(
        actions.try_recv(),
        Ok(DownloadAction::Remove {
            expected_session_id: 3,
            delete_file: false,
            ..
        })
    ));
    Ok(())
}

#[test]
fn active_download_still_asks_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let history = TempFile::new("delete-active-on");
    let (ui, state, mut actions, snapshots) = bound_window(true, &history)?;
    let snapshot = DownloadSnapshot {
        session_id: 3,
        status: DownloadStatus::Downloading,
        ..Default::default()
    };
    snapshots.send(snapshot.clone())?;
    *state.displayed_delete_target.borrow_mut() = DeleteTarget::displayed(&snapshot);

    ui.set_selected_row(0);
    ui.invoke_request_delete_selected();

    assert!(ui.get_show_delete_dialog());
    assert!(
        actions.try_recv().is_err(),
        "nothing is sent until confirmed"
    );
    Ok(())
}
