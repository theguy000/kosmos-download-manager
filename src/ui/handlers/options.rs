use super::sidebar::update_sidebar_categories;
use super::table::{resort, update_selection_state};
use crate::platform::{default_download_directory, set_startup_enabled, startup_enabled};
use crate::settings::{Category, SaveSettings};
use crate::ui::projection::history_table_item;
use crate::ui::state::AppState;
use crate::ui::view::{CategoryOption, ComboItem, MainWindow};
use slint::ComponentHandle;
use slint::Model;
use std::path::{Path, PathBuf};

fn options_snapshot(window: &MainWindow) -> Vec<CategoryOption> {
    window.get_options_categories().iter().collect()
}

fn set_options(window: &MainWindow, categories: &[CategoryOption]) {
    window.set_options_categories(slint::ModelRc::from(categories));
}

fn category_dir(categories: &[CategoryOption], category: Category) -> slint::SharedString {
    categories
        .get(category.category_id() as usize)
        .map(|option| option.dir.clone())
        .unwrap_or_default()
}

fn update_option(window: &MainWindow, category: Category, edit: impl FnOnce(&mut CategoryOption)) {
    let mut categories = options_snapshot(window);
    if let Some(option) = categories
        .iter_mut()
        .find(|option| option.id == category.category_id())
    {
        edit(option);
    }
    set_options(window, &categories);
}

fn category_action(enabled: bool) -> (&'static str, &'static str) {
    if enabled {
        ("close", "Disable category")
    } else {
        ("plus", "Enable category")
    }
}

/// Projects the current category rows into the generic combo's display items.
pub(crate) fn update_options_combo_items(window: &MainWindow) {
    let items: Vec<ComboItem> = window
        .get_options_categories()
        .iter()
        .map(|option| {
            let (action_icon, action_label) = if option.id == Category::General.category_id() {
                ("", "")
            } else {
                category_action(option.enabled)
            };
            ComboItem {
                text: option.name.clone(),
                action_icon: action_icon.into(),
                action_label: action_label.into(),
            }
        })
        .collect();
    window.set_options_combo_items(slint::ModelRc::from(items.as_slice()));
}

/// Loads the persisted categories into the dialog's working rows.
pub(crate) fn load_options_categories(window: &MainWindow, settings: &SaveSettings) {
    let categories: Vec<CategoryOption> = Category::ALL
        .iter()
        .map(|&category| CategoryOption {
            id: category.category_id(),
            name: category.display_name().into(),
            dir: settings
                .category_path(category)
                .to_string_lossy()
                .as_ref()
                .into(),
            file_types: settings.extensions_for(category).join(", ").into(),
            enabled: settings.is_category_active(category),
        })
        .collect();
    set_options(window, &categories);
    update_options_combo_items(window);
}

pub(crate) fn toggle_category_list(window: &MainWindow, category: Category) {
    if category == Category::General {
        return;
    }
    update_option(window, category, |option| option.enabled = !option.enabled);
    update_options_combo_items(window);
}

pub(crate) fn toggle_kdm_folder(window: &MainWindow, enabled: bool) {
    let categories = options_snapshot(window);
    let current_default = PathBuf::from(category_dir(&categories, Category::General).as_str());
    let new_default = SaveSettings::apply_kdm_folder(&current_default, enabled);

    if new_default != current_default {
        update_option(window, Category::General, |option| {
            option.dir = new_default.to_string_lossy().as_ref().into();
        });
        update_category_defaults(window, &current_default, &new_default);
    }
}

pub(crate) fn update_category_defaults(
    window: &MainWindow,
    old_default: &Path,
    new_default: &Path,
) {
    let mut categories = options_snapshot(window);
    for option in &mut categories {
        let Ok(category) = Category::try_from(option.id) else {
            continue;
        };
        if category == Category::General {
            continue;
        }
        let current_path = Path::new(option.dir.as_str());
        if option.dir.is_empty()
            || current_path == SaveSettings::default_subfolder(old_default, category)
        {
            option.dir = SaveSettings::default_subfolder(new_default, category)
                .to_string_lossy()
                .as_ref()
                .into();
        }
    }
    set_options(window, &categories);
}

pub(crate) fn bind_options_handlers(window: &MainWindow, state: &AppState) {
    {
        let window_weak = window.as_weak();
        let save_settings = state.save_settings.clone();
        window.on_options_opened(move || {
            if let Some(window) = window_weak.upgrade() {
                let settings = save_settings.borrow();
                load_options_categories(&window, &settings);
                window.set_options_use_kdm_folder(settings.use_kdm_folder);
            }

            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                let enabled = tokio::task::spawn_blocking(startup_enabled)
                    .await
                    .unwrap_or_default();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window_weak.upgrade() {
                        window.set_options_launch_on_startup(enabled);
                    }
                });
            });
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_toggle_options_category_list(move |cat_id| {
            let Ok(category) = Category::try_from(cat_id) else {
                return;
            };
            if let Some(window) = window_weak.upgrade() {
                toggle_category_list(&window, category);
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_toggle_options_kdm_folder(move |enabled| {
            if let Some(window) = window_weak.upgrade() {
                toggle_kdm_folder(&window, enabled);
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_default_dir_edited(move |old_default_str, new_default_str| {
            if let Some(window) = window_weak.upgrade() {
                let new_trimmed = new_default_str.trim();
                if !new_trimmed.is_empty() {
                    update_category_defaults(
                        &window,
                        Path::new(old_default_str.trim()),
                        Path::new(new_trimmed),
                    );
                }
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_browse_options_folder(move |cat_id| {
            let Ok(category) = Category::try_from(cat_id) else {
                return;
            };
            if let Some(window) = window_weak.upgrade() {
                let categories = options_snapshot(&window);
                let current_str = category_dir(&categories, category);
                let start_dir =
                    if !current_str.is_empty() && Path::new(current_str.as_str()).exists() {
                        PathBuf::from(current_str.as_str())
                    } else {
                        let def = category_dir(&categories, Category::General);
                        if !def.is_empty() && Path::new(def.as_str()).exists() {
                            PathBuf::from(def.as_str())
                        } else {
                            default_download_directory()
                        }
                    };

                if let Some(folder) = rfd::FileDialog::new()
                    .set_directory(&start_dir)
                    .pick_folder()
                {
                    let old_default = category_dir(&categories, Category::General);
                    update_option(&window, category, |option| {
                        option.dir = folder.to_string_lossy().as_ref().into();
                    });
                    if category == Category::General {
                        update_category_defaults(&window, Path::new(old_default.as_str()), &folder);
                    }
                }
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_reset_options_category_default(move |cat_id| {
            let Ok(category) = Category::try_from(cat_id) else {
                return;
            };
            if let Some(window) = window_weak.upgrade() {
                let categories = options_snapshot(&window);
                if category == Category::General {
                    let old_default = category_dir(&categories, Category::General);
                    let new_default = SaveSettings::apply_kdm_folder(
                        &default_download_directory(),
                        window.get_options_use_kdm_folder(),
                    );
                    update_option(&window, category, |option| {
                        option.dir = new_default.to_string_lossy().as_ref().into();
                    });
                    update_category_defaults(
                        &window,
                        Path::new(old_default.as_str()),
                        &new_default,
                    );
                } else {
                    let default_dir =
                        PathBuf::from(category_dir(&categories, Category::General).as_str());
                    update_option(&window, category, |option| {
                        option.dir = SaveSettings::default_subfolder(&default_dir, category)
                            .to_string_lossy()
                            .as_ref()
                            .into();
                    });
                }
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let save_settings = state.save_settings.clone();
        let history_store = state.history_store.clone();
        let download_history = state.download_history.clone();
        window.on_commit_options(move |enabled| {
            if let Some(window) = window_weak.upgrade() {
                let categories = options_snapshot(&window);
                let default_dir_raw = category_dir(&categories, Category::General);
                let default_dir_trimmed = default_dir_raw.trim();
                let use_kdm = window.get_options_use_kdm_folder();
                let effective_default = if default_dir_trimmed.is_empty() {
                    SaveSettings::apply_kdm_folder(&default_download_directory(), use_kdm)
                } else {
                    PathBuf::from(default_dir_trimmed)
                };

                let default_dir = if use_kdm {
                    SaveSettings::apply_kdm_folder(&effective_default, false)
                } else {
                    effective_default.clone()
                };

                let mut new_settings = SaveSettings {
                    default_dir,
                    use_kdm_folder: use_kdm,
                    ..Default::default()
                };
                for option in &categories {
                    if let Ok(category) = Category::try_from(option.id)
                        && category != Category::General
                    {
                        new_settings.set_category_dir(
                            category,
                            SaveSettings::category_override(
                                &option.dir,
                                &effective_default,
                                category,
                            ),
                        );
                        if let Some(extensions) =
                            SaveSettings::file_type_override(category, &option.file_types)
                        {
                            new_settings.file_types.insert(category, extensions);
                        }
                        new_settings.set_category_enabled(category, option.enabled);
                    }
                }

                *save_settings.borrow_mut() = new_settings.clone();
                update_sidebar_categories(&window, &new_settings);
                window.set_dest_dir_text(effective_default.to_string_lossy().as_ref().into());

                {
                    let settings = save_settings.borrow();
                    let store = history_store.borrow();
                    download_history.set_vec(
                        store
                            .entries()
                            .iter()
                            .map(|entry| history_table_item(&settings, entry))
                            .collect::<Vec<_>>(),
                    );
                    resort(
                        &download_history,
                        window.get_sort_column(),
                        window.get_sort_ascending(),
                    );
                    update_selection_state(&window, window.get_selected_row());
                }

                let window_weak_save = window_weak.clone();
                tokio::spawn(async move {
                    let saved = tokio::task::spawn_blocking(move || new_settings.save()).await;
                    let failure = match saved {
                        Ok(result) => result.err().map(|error| error.to_string()),
                        Err(error) => Some(error.to_string()),
                    };
                    if let Some(message) = failure {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak_save.upgrade() {
                                window.set_action_error_message(
                                    format!("Could not save options: {message}").into(),
                                );
                            }
                        });
                    }
                });
            }

            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                let outcome = tokio::task::spawn_blocking(move || {
                    set_startup_enabled(enabled).map_err(|error| {
                        (
                            format!("Could not update the startup setting: {error}"),
                            startup_enabled(),
                        )
                    })
                })
                .await
                .unwrap_or_else(|error| {
                    Err((
                        format!("Could not update the startup setting: {error}"),
                        enabled,
                    ))
                });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window_weak.upgrade()
                        && let Err((message, actual)) = outcome
                    {
                        window.set_options_launch_on_startup(actual);
                        window.set_action_error_message(message.into());
                    }
                });
            });
        });
    }
}
