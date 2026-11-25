//! The host's editor, not CADCraft's app: the product's name in every message, and the UI set up
//! as a pane of the host (menus inside the pane, no title row, no update check, no window of its
//! own). The binary applies it; the brand test (`tests/brand.rs`) applies the same and reads what
//! the interface shows.

use cadcraft_ui_egui::CadApp;

/// The product this editor is part of: the UI's messages name it where they name CADCraft
/// (`cadcraft_engine::product`).
pub const PRODUCT: &str = "CadZ";

/// Process-wide, before any message is made or read.
pub fn name_product() {
    cadcraft_engine::product::set_name(PRODUCT);
}

/// The app as a pane of the host: its menus drawn in the pane (macOS has no native menu bar for
/// it), no title row of its own, the interface in `lang` and `dark` or light.
pub fn editor(app: &mut CadApp, lang: &str, dark: bool) {
    // No native menu bar for the pane: the in-window menu is its only menu, kept by a reset.
    app.set_native_menus(false);
    app.integrated_titlebar = false;
    // Paste answers come in the next pass here (no Wayland clipboard of its own).
    app.set_clipboard_late_answers(false);
    app.ui.start_tab = false;
    if let Some(language) = cadcraft_ui_egui::i18n::Preference::parse(lang) {
        app.ui.interface_language = language;
    }
    app.system_languages = vec![lang.to_string()];
    app.ui.theme = if dark { cadcraft_ui_egui::theme::ThemePref::Dark } else { cadcraft_ui_egui::theme::ThemePref::Light };
}
