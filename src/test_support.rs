use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_FILE: AtomicUsize = AtomicUsize::new(0);

/// A unique path in the OS temp directory, removed on creation and on drop.
pub(crate) struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub(crate) fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kosmos-test-{name}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
