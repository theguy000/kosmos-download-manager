//! Persisted settings for download destination directories and categories.

mod category;
mod table;

pub use category::Category;
pub(crate) use table::TableColumnWidths;

use crate::platform::default_download_directory;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SETTINGS_FILE_NAME: &str = "save_settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSettings {
    pub default_dir: PathBuf,
    #[serde(default)]
    pub use_kdm_folder: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub category_dirs: BTreeMap<Category, PathBuf>,
    /// Extension lists that override the built-in defaults, keyed by category.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub file_types: BTreeMap<Category, Vec<String>>,
    /// Categories removed from the sidebar. Their saved file types stay so re-enabling restores
    /// the user's own list.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled_categories: BTreeSet<Category>,
}

impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            default_dir: default_download_directory(),
            use_kdm_folder: false,
            category_dirs: BTreeMap::new(),
            file_types: BTreeMap::new(),
            disabled_categories: BTreeSet::new(),
        }
    }
}

impl SaveSettings {
    #[must_use]
    pub const fn category_subfolder_name(category: Category) -> &'static str {
        match category {
            Category::General => "",
            Category::Compressed => "Compressed",
            Category::Documents => "Documents",
            Category::Music => "Music",
            Category::Programs => "Programs",
            Category::Video => "Video",
            Category::Images => "Images",
            Category::Ebooks => "Ebooks",
            Category::SourceCode => "Source Code",
            Category::DiskImages => "Disk Images",
            Category::Torrents => "Torrents",
            Category::Databases => "Databases",
        }
    }

    #[must_use]
    pub fn default_subfolder(default_dir: &Path, category: Category) -> PathBuf {
        let sub = Self::category_subfolder_name(category);
        if sub.is_empty() {
            default_dir.to_path_buf()
        } else {
            default_dir.join(sub)
        }
    }

    /// The directory a category overrides its default subfolder with, if any.
    fn custom_dir(&self, category: Category) -> Option<&Path> {
        self.category_dirs.get(&category).map(PathBuf::as_path)
    }

    pub const KDM_FOLDER_NAME: &str = "KDM";

    /// Applies or strips the KDM folder suffix from `path` based on `enabled`.
    #[must_use]
    pub fn apply_kdm_folder(path: &Path, enabled: bool) -> PathBuf {
        if enabled {
            if path.ends_with(Self::KDM_FOLDER_NAME) {
                path.to_path_buf()
            } else {
                path.join(Self::KDM_FOLDER_NAME)
            }
        } else if path.ends_with(Self::KDM_FOLDER_NAME) {
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(path)
                .to_path_buf()
        } else {
            path.to_path_buf()
        }
    }

    /// The base directory all default downloads route through, taking `use_kdm_folder` into account.
    #[must_use]
    pub fn effective_default_dir(&self) -> PathBuf {
        Self::apply_kdm_folder(&self.default_dir, self.use_kdm_folder)
    }

    /// Points a category at a custom directory, or back at its default subfolder with `None`.
    pub fn set_category_dir(&mut self, category: Category, dir: Option<PathBuf>) {
        if category == Category::General {
            return;
        }
        if let Some(dir) = dir {
            self.category_dirs.insert(category, dir);
        } else {
            self.category_dirs.remove(&category);
        }
    }

    /// Returns the effective directory for the given category.
    #[must_use]
    pub fn category_path(&self, category: Category) -> PathBuf {
        self.custom_dir(category).map_or_else(
            || Self::default_subfolder(&self.effective_default_dir(), category),
            Path::to_path_buf,
        )
    }

    /// The effective extensions for a category: the saved override, or the built-in defaults.
    #[must_use]
    pub fn extensions_for(&self, category: Category) -> Vec<String> {
        self.file_types.get(&category).map_or_else(
            || {
                Category::extensions(category)
                    .iter()
                    .map(|extension| (*extension).to_string())
                    .collect()
            },
            Clone::clone,
        )
    }

    /// Returns whether the category takes part in routing and appears in the sidebar.
    #[must_use]
    pub fn is_category_active(&self, category: Category) -> bool {
        category == Category::General || !self.disabled_categories.contains(&category)
    }

    /// Enables or disables a category without touching its saved file types.
    pub fn set_category_enabled(&mut self, category: Category, enabled: bool) {
        if category == Category::General {
            return;
        }
        if enabled {
            self.disabled_categories.remove(&category);
        } else {
            self.disabled_categories.insert(category);
        }
    }

    /// Parses a comma, semicolon, or whitespace separated extension list into unique lowercase
    /// tokens without surrounding dots.
    #[must_use]
    pub fn normalize_extensions(text: &str) -> Vec<String> {
        let mut extensions: Vec<String> = Vec::new();
        for token in text.split(|character: char| {
            character == ',' || character == ';' || character.is_whitespace()
        }) {
            let extension = token.trim_matches('.').to_ascii_lowercase();
            if !extension.is_empty() && !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        extensions
    }

    /// Returns the override to persist for a category, or `None` when the parsed list matches
    /// the built-in defaults.
    #[must_use]
    pub fn file_type_override(category: Category, text: &str) -> Option<Vec<String>> {
        let extensions = Self::normalize_extensions(text);
        let defaults = Category::extensions(category);
        let differs = extensions.len() != defaults.len()
            || extensions
                .iter()
                .zip(defaults)
                .any(|(ext, &default_ext)| ext.as_str() != default_ext);
        differs.then_some(extensions)
    }

    /// The category a filename routes to, honoring the saved file type overrides.
    #[must_use]
    pub fn category_for_filename(&self, filename: &str) -> Category {
        let extension = extension_of(filename);
        if extension.is_empty() {
            return Category::General;
        }
        Category::ALL
            .into_iter()
            .find(|category| self.has_extension(*category, extension))
            .unwrap_or(Category::General)
    }

    fn has_extension(&self, category: Category, extension: &str) -> bool {
        if !self.is_category_active(category) {
            return false;
        }
        self.file_types.get(&category).map_or_else(
            || {
                Category::extensions(category)
                    .iter()
                    .any(|saved| saved.eq_ignore_ascii_case(extension))
            },
            |extensions| {
                extensions
                    .iter()
                    .any(|saved| saved.eq_ignore_ascii_case(extension))
            },
        )
    }

    /// Determines the save folder for a download URL.
    #[must_use]
    pub fn path_for_url(&self, url: &str) -> PathBuf {
        let filename = crate::client::extract_filename(url, None);
        let category = self.category_for_filename(&filename);
        self.category_path(category)
    }

    /// Checks if a path matches the default directory or any enabled category path.
    #[must_use]
    pub fn is_managed_path(&self, path: &Path) -> bool {
        path == self.default_dir
            || path == self.effective_default_dir()
            || Category::ALL.iter().any(|&category| {
                category != Category::General
                    && self.is_category_active(category)
                    && path == self.category_path(category)
            })
    }

    /// Returns the directory a category overrides the default with, or `None` when it stays
    /// on its default subfolder.
    #[must_use]
    pub fn category_override(
        value: &str,
        default_dir: &Path,
        category: Category,
    ) -> Option<PathBuf> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        let path = PathBuf::from(trimmed);
        (path != Self::default_subfolder(default_dir, category)).then_some(path)
    }

    pub fn load() -> Self {
        Self::load_from(&settings_path()).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut settings: Self = serde_json::from_str(&text).ok()?;
        if settings.default_dir.as_os_str().is_empty() {
            settings.default_dir = default_download_directory();
        }
        settings
            .category_dirs
            .retain(|_, path| !path.as_os_str().is_empty());
        settings.disabled_categories.remove(&Category::General);
        Some(settings)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&settings_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::fs::write_atomic(path, json.as_bytes())
    }
}

fn extension_of(filename: &str) -> &str {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

fn settings_path() -> PathBuf {
    crate::platform::data_directory().join(SETTINGS_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("kosmos_save_test_{}", std::process::id()));
        let path = dir.join("save_settings.json");

        let original = SaveSettings {
            default_dir: PathBuf::from("D:\\Downloads"),
            use_kdm_folder: false,
            category_dirs: BTreeMap::from([
                (
                    Category::Compressed,
                    PathBuf::from("D:\\Downloads\\CustomArchives"),
                ),
                (Category::Music, PathBuf::from("D:\\Music")),
            ]),
            file_types: BTreeMap::from([(Category::Video, vec!["webm".to_string()])]),
            disabled_categories: BTreeSet::from([Category::Torrents]),
        };

        assert!(original.save_to(&path).is_ok());
        let loaded = SaveSettings::load_from(&path);
        assert_eq!(loaded, Some(original));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_default_subfolders() {
        let settings = SaveSettings {
            default_dir: PathBuf::from("C:\\Users\\test\\Downloads"),
            ..Default::default()
        };

        assert_eq!(
            settings.category_path(Category::General),
            PathBuf::from("C:\\Users\\test\\Downloads")
        );
        assert_eq!(
            settings.category_path(Category::Compressed),
            PathBuf::from("C:\\Users\\test\\Downloads").join("Compressed")
        );
        assert_eq!(
            settings.category_path(Category::Video),
            PathBuf::from("C:\\Users\\test\\Downloads").join("Video")
        );
    }

    #[test]
    fn test_path_for_url_routing() {
        let settings = SaveSettings {
            default_dir: PathBuf::from("/home/user/Downloads"),
            ..Default::default()
        };

        assert_eq!(
            settings.path_for_url("https://example.com/test.zip"),
            PathBuf::from("/home/user/Downloads/Compressed")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/movie.mp4"),
            PathBuf::from("/home/user/Downloads/Video")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/song.mp3"),
            PathBuf::from("/home/user/Downloads/Music")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/doc.pdf"),
            PathBuf::from("/home/user/Downloads/Documents")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/app.exe"),
            PathBuf::from("/home/user/Downloads/Programs")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/unknown_file"),
            PathBuf::from("/home/user/Downloads")
        );
    }

    #[test]
    fn test_category_override_keeps_explicit_old_default_subfolder() {
        let default_dir = PathBuf::from("D:\\Downloads");

        assert_eq!(
            SaveSettings::category_override(
                &default_dir.join("Compressed").to_string_lossy(),
                &default_dir,
                Category::Compressed
            ),
            None,
            "The current default subfolder is not an override"
        );
        assert_eq!(
            SaveSettings::category_override(
                "C:\\Old\\Downloads\\Compressed",
                &default_dir,
                Category::Compressed
            ),
            Some(PathBuf::from("C:\\Old\\Downloads\\Compressed")),
            "A folder from an earlier default directory stays an explicit override"
        );
        assert_eq!(
            SaveSettings::category_override("   ", &default_dir, Category::Video),
            None,
            "An empty field falls back to the default subfolder"
        );
    }

    #[test]
    fn test_category_try_from_id() {
        for (index, category) in Category::ALL.iter().enumerate() {
            assert_eq!(Category::try_from(index as i32), Ok(*category));
            assert_eq!(category.category_id(), index as i32);
            assert_ne!(category.as_str(), "");
            assert_ne!(category.display_name(), "");
        }
        assert_eq!(Category::try_from(99), Err(99));
        assert_eq!(Category::try_from(-1), Err(-1));
    }

    #[test]
    fn test_is_managed_path() {
        let default_dir = PathBuf::from("D:\\Downloads");
        let mut settings = SaveSettings {
            default_dir: default_dir.clone(),
            ..Default::default()
        };
        settings.set_category_dir(
            Category::Compressed,
            Some(PathBuf::from("D:\\CustomArchives")),
        );

        // Default dir itself
        assert!(settings.is_managed_path(&default_dir));
        // Custom directory
        assert!(settings.is_managed_path(Path::new("D:\\CustomArchives")));
        // Default subfolder for Documents
        assert!(settings.is_managed_path(&default_dir.join("Documents")));
        // Default subfolder for Video
        assert!(settings.is_managed_path(&default_dir.join("Video")));
        // Overridden default subfolder is not matched as custom is active
        assert!(!settings.is_managed_path(&default_dir.join("Compressed")));
        // Completely unrelated path
        assert!(!settings.is_managed_path(Path::new("D:\\Other\\Folder")));
    }

    #[test]
    fn test_file_type_overrides_route_and_classify() {
        let default_dir = PathBuf::from("D:\\Downloads");
        let mut settings = SaveSettings {
            default_dir: default_dir.clone(),
            ..Default::default()
        };
        settings
            .file_types
            .insert(Category::Video, vec!["webm".into()]);

        assert_eq!(settings.category_for_filename("clip.webm"), Category::Video);
        assert_eq!(
            settings.path_for_url("https://example.com/clip.webm"),
            default_dir.join("Video")
        );
        assert_eq!(
            settings.category_for_filename("movie.mp4"),
            Category::General,
            "A removed extension falls back to the default category"
        );
    }

    #[test]
    fn test_normalize_extensions() {
        assert_eq!(
            SaveSettings::normalize_extensions(" .ZIP, rar;7z  7z "),
            vec!["zip", "rar", "7z"]
        );
        assert_eq!(
            SaveSettings::normalize_extensions(" , ; "),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_file_type_override_only_persists_changes() {
        let defaults = Category::extensions(Category::Video).join(", ");
        assert_eq!(
            SaveSettings::file_type_override(Category::Video, &defaults),
            None
        );
        assert_eq!(
            SaveSettings::file_type_override(Category::Video, "WebM .mkv"),
            Some(vec!["webm".into(), "mkv".into()])
        );
        assert_eq!(
            SaveSettings::file_type_override(Category::Music, ""),
            Some(Vec::new()),
            "An empty list is a valid override that routes no extensions"
        );
    }

    #[test]
    fn test_category_enabled_state() {
        let mut settings = SaveSettings::default();
        assert!(settings.is_category_active(Category::General));
        assert!(settings.is_category_active(Category::Compressed));

        settings.set_category_enabled(Category::Compressed, false);
        assert!(!settings.is_category_active(Category::Compressed));
        assert_eq!(
            settings.category_for_filename("archive.zip"),
            Category::General
        );
        assert!(
            settings
                .extensions_for(Category::Compressed)
                .contains(&"zip".into()),
            "Disabling a category keeps its file types"
        );

        settings.set_category_enabled(Category::Compressed, true);
        assert!(settings.is_category_active(Category::Compressed));
        assert_eq!(
            settings.category_for_filename("archive.zip"),
            Category::Compressed
        );

        settings.set_category_enabled(Category::General, false);
        assert!(settings.is_category_active(Category::General));
        assert!(settings.disabled_categories.is_empty());
    }

    #[test]
    fn test_disabled_category_keeps_custom_list_and_drops_managed_path() {
        let mut settings = SaveSettings {
            default_dir: PathBuf::from("D:\\Downloads"),
            ..Default::default()
        };
        settings
            .file_types
            .insert(Category::Compressed, vec!["x".into()]);
        settings.set_category_enabled(Category::Compressed, false);

        assert_eq!(
            settings.extensions_for(Category::Compressed),
            vec!["x".to_string()]
        );
        assert_eq!(
            settings.category_for_filename("archive.x"),
            Category::General
        );
        assert!(
            !settings.is_managed_path(Path::new("D:\\Downloads\\Compressed")),
            "A disabled category's folder is no longer managed"
        );
    }

    #[test]
    fn test_kdm_folder_routing() {
        let base_dir = PathBuf::from("D:\\Downloads");
        let mut settings = SaveSettings {
            default_dir: base_dir.clone(),
            use_kdm_folder: false,
            ..Default::default()
        };

        assert_eq!(settings.effective_default_dir(), base_dir);
        assert_eq!(settings.category_path(Category::General), base_dir);
        assert_eq!(
            settings.category_path(Category::Compressed),
            base_dir.join("Compressed")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/test.zip"),
            base_dir.join("Compressed")
        );

        settings.use_kdm_folder = true;
        let kdm_dir = base_dir.join("KDM");
        assert_eq!(settings.effective_default_dir(), kdm_dir);
        assert_eq!(settings.category_path(Category::General), kdm_dir);
        assert_eq!(
            settings.category_path(Category::Compressed),
            kdm_dir.join("Compressed")
        );
        assert_eq!(
            settings.category_path(Category::Video),
            kdm_dir.join("Video")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/test.zip"),
            kdm_dir.join("Compressed")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/movie.mp4"),
            kdm_dir.join("Video")
        );
        assert_eq!(
            settings.path_for_url("https://example.com/unknown.xyz"),
            kdm_dir
        );

        assert!(settings.is_managed_path(&kdm_dir));
        assert!(settings.is_managed_path(&kdm_dir.join("Compressed")));
        assert!(settings.is_managed_path(&base_dir));

        settings.use_kdm_folder = false;
        assert_eq!(settings.effective_default_dir(), base_dir);
        assert_eq!(
            settings.path_for_url("https://example.com/test.zip"),
            base_dir.join("Compressed")
        );
    }
}
