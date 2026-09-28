use crate::settings::{Category, SaveSettings};
use crate::ui::view::{MainWindow, Navigation, Subcategory};
use slint::ComponentHandle;

/// Rebuilds the sidebar list from the categories that are enabled in `settings`, dropping the
/// selection when it points at a category that is no longer listed.
pub(crate) fn update_sidebar_categories(window: &MainWindow, settings: &SaveSettings) {
    let subcategories: Vec<Subcategory> = Category::ALL
        .iter()
        .filter(|&&category| category != Category::General && settings.is_category_active(category))
        .map(|&category| Subcategory {
            id: category.category_id(),
            name: category.display_name().into(),
            icon: category.as_str().into(),
        })
        .collect();
    let current_selected = window.get_selected_category();
    if let Ok(category) = Category::try_from(current_selected)
        && !settings.is_category_active(category)
    {
        window.set_selected_category(window.global::<Navigation>().get_all());
    }
    window.set_sidebar_subcategories(slint::ModelRc::from(subcategories.as_slice()));
}
