use super::support::install_test_platform;
use crate::engine::ChunkSnapshot;
use crate::settings::SaveSettings;
use crate::ui::handlers::actions::send_action;
use crate::ui::projection::{chunk_progress, should_project_snapshot, update_window_state};
use crate::ui::view::MainWindow;

#[test]
fn snapshot_projection_handles_initial_changed_and_closed_updates() {
    let (tx, mut rx) = tokio::sync::watch::channel(0_u8);
    let mut initial = true;

    {
        let snapshot = rx.borrow_and_update();
        assert!(should_project_snapshot(&snapshot, &mut initial));
        assert_eq!(*snapshot, 0);
    }
    {
        let snapshot = rx.borrow_and_update();
        assert!(!should_project_snapshot(&snapshot, &mut initial));
    }

    assert!(tx.send(1).is_ok());
    {
        let snapshot = rx.borrow_and_update();
        assert!(should_project_snapshot(&snapshot, &mut initial));
        assert_eq!(*snapshot, 1);
    }
    {
        let snapshot = rx.borrow_and_update();
        assert!(!should_project_snapshot(&snapshot, &mut initial));
    }

    assert!(tx.send(2).is_ok());
    drop(tx);
    assert!(rx.has_changed().is_err());
    {
        let snapshot = rx.borrow_and_update();
        assert!(should_project_snapshot(&snapshot, &mut initial));
        assert_eq!(*snapshot, 2);
    }
    {
        let snapshot = rx.borrow_and_update();
        assert!(!should_project_snapshot(&snapshot, &mut initial));
    }
}

#[test]
fn test_duplicate_prompt_projection_updates_window() -> Result<(), Box<dyn std::error::Error>> {
    let _ = install_test_platform()?;
    let ui = MainWindow::new()?;
    assert!(!ui.get_show_duplicate_dialog());

    let mut snap = crate::engine::DownloadSnapshot {
        session_id: 10,
        url: "http://example.com/file.zip".into(),
        filename: "file.zip".into(),
        status: crate::engine::DownloadStatus::Connecting,
        duplicate: Some(crate::engine::DuplicatePrompt {
            session_id: 10,
            url: "http://example.com/file.zip".into(),
            filename: "file.zip".into(),
            existing_bytes: Some(2048),
            link_duplicate: false,
        }),
        ..Default::default()
    };

    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert!(ui.get_show_duplicate_dialog());
    assert_eq!(ui.get_duplicate_url(), "http://example.com/file.zip");
    assert_eq!(ui.get_duplicate_filename(), "file.zip");
    assert_eq!(ui.get_duplicate_existing_size(), "2.0 KB");
    assert!(!ui.get_duplicate_is_link());
    assert_eq!(ui.get_duplicate_selected_option(), 0);
    assert!(!ui.get_duplicate_remember());

    ui.set_duplicate_selected_option(1);
    ui.set_duplicate_remember(true);

    // Another update tick for the same prompt preserves user choice
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert_eq!(ui.get_duplicate_selected_option(), 1);
    assert!(ui.get_duplicate_remember());

    snap.duplicate = None;
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert!(!ui.get_show_duplicate_dialog());

    // Adversarial verification: send_action channel capacity saturation
    let (tx, _rx) = tokio::sync::mpsc::channel(32);
    for _ in 0..32 {
        let ok = send_action(
            &ui,
            &tx,
            crate::engine::DownloadAction::ResolveDuplicate {
                session_id: 1,
                choice: None,
            },
        );
        assert!(ok);
        assert_eq!(ui.get_action_error_message(), "");
    }
    let ok_33 = send_action(
        &ui,
        &tx,
        crate::engine::DownloadAction::ResolveDuplicate {
            session_id: 1,
            choice: None,
        },
    );
    assert!(
        !ok_33,
        "Expected 33rd action to fail due to channel capacity"
    );
    assert_eq!(
        ui.get_action_error_message(),
        "Download engine is busy. Try the action again."
    );

    snap.downloaded_bytes = 1024;
    snap.total_bytes = Some(4096);
    snap.status = crate::engine::DownloadStatus::Downloading;
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert_eq!(ui.get_active_size(), "1.0 KB / 4.0 KB");

    snap.status = crate::engine::DownloadStatus::Completed;
    snap.downloaded_bytes = 4096;
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert_eq!(ui.get_active_size(), "4.0 KB");

    Ok(())
}

#[test]
fn test_chunk_snapshot_projection_updates_window() -> Result<(), Box<dyn std::error::Error>> {
    use slint::Model;

    let _ = install_test_platform()?;
    let ui = MainWindow::new()?;

    let snap = crate::engine::DownloadSnapshot {
        session_id: 1,
        url: "http://example.com/file.zip".into(),
        filename: "file.zip".into(),
        status: crate::engine::DownloadStatus::Downloading,
        total_bytes: Some(4000),
        downloaded_bytes: 2000,
        chunks: vec![
            crate::engine::ChunkSnapshot {
                id: 0,
                downloaded: 1000,
                total: 2000,
                is_done: false,
            },
            crate::engine::ChunkSnapshot {
                id: 1,
                downloaded: 2000,
                total: 2000,
                is_done: true,
            },
        ],
        ..Default::default()
    };

    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert!(ui.get_is_downloading());

    let chunks = ui.get_active_chunks();
    assert_eq!(chunks.row_count(), 2);
    let c0 = chunks.row_data(0).unwrap();
    assert_eq!(c0.id, 0);
    assert_eq!(c0.progress, 0.5);
    assert!(!c0.is_done);

    let c1 = chunks.row_data(1).unwrap();
    assert_eq!(c1.id, 1);
    assert_eq!(c1.progress, 1.0);
    assert!(c1.is_done);

    Ok(())
}

#[test]
fn chunk_progress_handles_boundaries() {
    let chunk = |downloaded, total, is_done| ChunkSnapshot {
        id: 0,
        downloaded,
        total,
        is_done,
    };
    assert_eq!(chunk_progress(&chunk(10, 0, false)), 0.0);
    assert_eq!(chunk_progress(&chunk(300, 200, false)), 1.0);
    assert_eq!(chunk_progress(&chunk(0, 200, true)), 1.0);
    assert_eq!(chunk_progress(&chunk(50, 200, false)), 0.25);
}

#[test]
fn chunk_projection_updates_in_place_and_clears() -> Result<(), Box<dyn std::error::Error>> {
    use slint::Model;

    let _ = install_test_platform()?;
    let ui = MainWindow::new()?;
    let chunk = |downloaded| ChunkSnapshot {
        id: 0,
        downloaded,
        total: 100,
        is_done: false,
    };
    let mut snap = crate::engine::DownloadSnapshot {
        chunks: vec![chunk(10), chunk(20)],
        ..Default::default()
    };
    update_window_state(&ui, &SaveSettings::default(), &snap);
    let model = ui.get_active_chunks();

    snap.chunks = vec![chunk(50), chunk(20)];
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert_eq!(model.row_data(0).map(|c| c.progress), Some(0.5));
    assert_eq!(ui.get_active_chunks().row_count(), 2);

    snap.chunks.clear();
    update_window_state(&ui, &SaveSettings::default(), &snap);
    assert_eq!(ui.get_active_chunks().row_count(), 0);

    Ok(())
}
