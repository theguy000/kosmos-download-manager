use crate::history::HistoryEntry;
use crate::settings::Category;
use crate::ui::view::{CategoryOption, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Clipboard, Platform, WindowAdapter};
use slint::{Model, ModelRc};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub(super) fn finished_entry(id: i32, filename: &str, total_bytes: u64) -> HistoryEntry {
    HistoryEntry {
        id,
        url: format!("https://example.com/{filename}"),
        filename: filename.to_string(),
        save_path: PathBuf::from(filename),
        total_bytes,
        completed_unix_ms: 1_700_000_000_000 + id as u64,
        description: String::new(),
    }
}

/// The dialog's default working rows, mirroring what Rust loads from default settings.
pub(super) fn default_option_categories() -> Vec<CategoryOption> {
    Category::ALL
        .iter()
        .map(|&category| CategoryOption {
            id: category.category_id(),
            name: category.display_name().into(),
            dir: slint::SharedString::new(),
            file_types: category.extensions().join(", ").into(),
            enabled: true,
        })
        .collect()
}

fn ensure_categories(ui: &MainWindow) -> Vec<CategoryOption> {
    let categories: Vec<CategoryOption> = ui.get_options_categories().iter().collect();
    if categories.is_empty() {
        default_option_categories()
    } else {
        categories
    }
}

fn update_category(ui: &MainWindow, category: usize, edit: impl FnOnce(&mut CategoryOption)) {
    let mut categories = ensure_categories(ui);
    if let Some(option) = categories.get_mut(category) {
        edit(option);
    }
    ui.set_options_categories(ModelRc::from(categories.as_slice()));
}

pub(super) fn option_dir(ui: &MainWindow, category: usize) -> slint::SharedString {
    ui.get_options_categories()
        .row_data(category)
        .map(|option| option.dir)
        .unwrap_or_default()
}

pub(super) fn set_option_dir(ui: &MainWindow, category: usize, value: &str) {
    update_category(ui, category, |option| option.dir = value.into());
}

pub(super) fn option_file_types(ui: &MainWindow, category: usize) -> slint::SharedString {
    ui.get_options_categories()
        .row_data(category)
        .map(|option| option.file_types)
        .unwrap_or_default()
}

pub(super) fn set_option_file_types(ui: &MainWindow, category: usize, value: &str) {
    update_category(ui, category, |option| option.file_types = value.into());
}

pub(super) fn option_enabled(ui: &MainWindow, category: usize) -> bool {
    ui.get_options_categories()
        .row_data(category)
        .is_some_and(|option| option.enabled)
}

pub(super) type TestContext = (Rc<MinimalSoftwareWindow>, Rc<RefCell<String>>);

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<RefCell<String>>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if clipboard == Clipboard::DefaultClipboard {
            *self.1.borrow_mut() = text.into();
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        (clipboard == Clipboard::DefaultClipboard).then(|| self.1.borrow().clone())
    }
}

thread_local! {
    static TEST_CONTEXT: RefCell<Option<TestContext>> = const { RefCell::new(None) };
}

// Slint platforms are thread-local and can only be set once per thread.
pub(super) fn install_test_platform() -> Result<TestContext, Box<dyn std::error::Error>> {
    if let Some((window, clipboard)) = TEST_CONTEXT.with(|slot| slot.borrow().clone()) {
        return Ok((window, clipboard));
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let clipboard = Rc::new(RefCell::new(String::new()));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), clipboard.clone())))?;
    TEST_CONTEXT.with(|slot| *slot.borrow_mut() = Some((window.clone(), clipboard.clone())));
    Ok((window, clipboard))
}
