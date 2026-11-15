//! The look of the app: dark or light, with a light touch of the brand
//! colours, the same as Duck Quill and Duck Forge Studio.

use eframe::egui::{
    self, Button, Color32, FontFamily, FontId, RichText, Stroke, TextStyle, Theme, ThemePreference,
};

/// Night indigo: what is chosen.
pub const NIGHT_INDIGO: Color32 = Color32::from_rgb(0x55, 0x46, 0xD0);
/// Night indigo made lighter, for headings that must read on the dark
/// and the light theme alike.
pub const NIGHT_INDIGO_TEXT: Color32 = Color32::from_rgb(0x8A, 0x7E, 0xF0);
/// Duck yellow: the main button.
pub const DUCK_YELLOW: Color32 = Color32::from_rgb(0xFF, 0xCC, 0x2E);
/// Ink navy: text on duck yellow.
pub const INK_NAVY: Color32 = Color32::from_rgb(0x1B, 0x14, 0x46);
/// Echo cyan: the sound waves of the icon.
pub const ECHO_CYAN: Color32 = Color32::from_rgb(0x46, 0xD6, 0xFF);
/// "Needs a look", and a warning.
pub const LOOK: Color32 = Color32::from_rgb(0xF0, 0xA0, 0x30);

/// The size of small text (descriptions, notes under a card), in points.
/// egui's own small size is 9, which is too small to read; 12 is the
/// size of egui's other secondary text.
pub const SMALL_TEXT: f32 = 12.0;

/// Give both the dark and the light look the brand's selection colour, and
/// small text that can be read. Done once at start: egui keeps a style for
/// each theme.
pub fn apply_brand(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.visuals.selection.bg_fill = NIGHT_INDIGO;
            style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            style.text_styles.insert(
                TextStyle::Small,
                FontId::new(SMALL_TEXT, FontFamily::Proportional),
            );
        });
    }
}

/// The main button of a screen or a dialog: duck yellow with navy text.
pub fn primary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text).color(INK_NAVY).strong()).fill(DUCK_YELLOW)
}

/// What "follow the system" is called here.
pub fn system_label() -> &'static str {
    if cfg!(windows) {
        "Same as Windows"
    } else {
        "Same as the system"
    }
}

/// The Theme menu: dark, light, or whatever the system is set to.
/// Returns the new choice when it changed.
pub fn theme_menu(ui: &mut egui::Ui) -> Option<ThemePreference> {
    let ctx = ui.ctx().clone();
    let mut choice = ctx.options(|o| o.theme_preference);
    let before = choice;
    ui.menu_button("Theme", |ui| {
        ui.radio_value(&mut choice, ThemePreference::Dark, "Dark");
        ui.radio_value(&mut choice, ThemePreference::Light, "Light");
        ui.radio_value(&mut choice, ThemePreference::System, system_label());
    });
    if choice != before {
        ctx.set_theme(choice);
        return Some(choice);
    }
    None
}

/// The theme saved in the settings ("dark", "light", "system"); dark if
/// none was saved.
pub fn from_name(name: Option<&str>) -> ThemePreference {
    match name {
        Some("light") => ThemePreference::Light,
        Some("system") => ThemePreference::System,
        _ => ThemePreference::Dark,
    }
}

/// The name a theme is saved under.
pub fn name(theme: ThemePreference) -> &'static str {
    match theme {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_survives_its_name() {
        for theme in [
            ThemePreference::Dark,
            ThemePreference::Light,
            ThemePreference::System,
        ] {
            assert_eq!(from_name(Some(name(theme))), theme);
        }
        assert_eq!(from_name(None), ThemePreference::Dark);
        assert_eq!(from_name(Some("purple")), ThemePreference::Dark);
    }
}
