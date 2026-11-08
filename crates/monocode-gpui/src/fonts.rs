use gpui::{App, Font, FontFallbacks};
use std::sync::{Arc, OnceLock};

#[derive(Default)]
struct InstalledFonts(OnceLock<Arc<Vec<String>>>);
impl gpui::Global for InstalledFonts {}

pub(crate) fn installed(cx: &App) -> Arc<Vec<String>> {
    let load = || {
        let mut names = cx.text_system().all_font_names();
        names.sort();
        names.dedup();
        Arc::new(names)
    };
    cx.try_global::<InstalledFonts>()
        .map_or_else(load, |cache| cache.0.get_or_init(load).clone())
}

/// GPUI resolves this family to the platform system UI font.
pub(crate) const UI: &str = ".SystemUIFont";

/// GPUI matches family names literally outside macOS, so the CSS generic
/// `monospace` resolves to nothing there and text falls back to a
/// proportional sans. Mirror the web build's terminal stack instead.
const MONO_CANDIDATES: &[&str] = &[
    "JetBrainsMono NFM",
    "JetBrainsMono Nerd Font Mono",
    "JetBrainsMono NF",
    "MesloLGS NF",
    "MesloLGS Nerd Font Mono",
    "FiraCode Nerd Font Mono",
    "Hack Nerd Font Mono",
    "CaskaydiaCove Nerd Font Mono",
    "JetBrains Mono",
    "Cascadia Mono",
    "Consolas",
    "DejaVu Sans Mono",
    "Ubuntu Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Courier New",
];

static MONO: OnceLock<String> = OnceLock::new();

/// The monospace family for code and the terminal.
pub(crate) fn mono() -> &'static str {
    MONO.get().map_or(
        if cfg!(target_os = "macos") {
            "Menlo"
        } else {
            "monospace"
        },
        String::as_str,
    )
}

/// The terminal's lead faces, as the web build's `--font-terminal`: a Nerd
/// Font patched JetBrains Mono keeps prompt text and icons in one face.
const TERMINAL_PRIMARY: &[&str] = &[
    "JetBrainsMono NFM",
    "JetBrainsMono Nerd Font Mono",
    "JetBrainsMono NF",
];
/// Fill the Private Use Area glyphs (powerline arrows, git and folder icons)
/// a plain monospace lacks; the system cascade never reaches them.
const TERMINAL_ICONS: &[&str] = &[
    "Symbols Nerd Font Mono",
    "Symbols Nerd Font",
    "MesloLGS NF",
    "MesloLGS Nerd Font Mono",
    "FiraCode Nerd Font Mono",
    "Hack Nerd Font Mono",
    "CaskaydiaCove Nerd Font Mono",
];

static TERMINAL: OnceLock<Font> = OnceLock::new();

/// The terminal's font: a Nerd Font when installed, else the code monospace,
/// with any installed Nerd Font as the fallback for icon glyphs.
pub(crate) fn terminal(cx: &App) -> Font {
    let mut font = TERMINAL
        .get_or_init(|| terminal_font(&installed(cx), mono()))
        .clone();
    if let Some(preferences) = cx.try_global::<crate::settings::Preferences>() {
        font.family = configured_family(
            &preferences.terminal_font_family,
            &installed(cx),
            &font.family,
        )
        .into();
    }
    font
}

fn terminal_font(installed: &[String], mono: &str) -> Font {
    let has = |name: &str| installed.iter().any(|font| font == name);
    let family = TERMINAL_PRIMARY
        .iter()
        .copied()
        .find(|name| has(name))
        .unwrap_or(mono);
    let icons: Vec<String> = TERMINAL_ICONS
        .iter()
        .filter(|name| has(name) && **name != family)
        .map(|name| name.to_string())
        .collect();
    let mut font = gpui::font(family.to_owned());
    font.fallbacks = (!icons.is_empty()).then(|| FontFallbacks::from_fonts(icons));
    font
}

pub(crate) fn init(cx: &mut App) {
    if cx.try_global::<InstalledFonts>().is_none() {
        cx.set_global(InstalledFonts::default());
    }
    if cfg!(target_os = "macos") {
        return;
    }
    if let Some(family) = pick_mono(&installed(cx), system_mono) {
        let _ = MONO.set(family);
    }
}

fn pick_mono(installed: &[String], system: impl FnOnce() -> Option<String>) -> Option<String> {
    let has = |name: &str| installed.iter().any(|font| font == name);
    MONO_CANDIDATES
        .iter()
        .find(|name| has(name))
        .map(|name| name.to_string())
        .or_else(|| system().filter(|name| has(name)))
}

/// What fontconfig maps the `monospace` alias to, as WebKitGTK does.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn system_mono() -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{family[0]}", "monospace"])
        .output()
        .ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !name.is_empty()).then_some(name)
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn system_mono() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::{pick_mono, terminal_font};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn prefers_the_web_stack_order() {
        let installed = names(&["Ubuntu", "DejaVu Sans Mono", "JetBrainsMono Nerd Font Mono"]);
        assert_eq!(
            pick_mono(&installed, || None).as_deref(),
            Some("JetBrainsMono Nerd Font Mono")
        );
    }

    #[test]
    fn falls_back_to_the_installed_system_alias() {
        let installed = names(&["Ubuntu", "Iosevka"]);
        assert_eq!(
            pick_mono(&installed, || Some("Iosevka".into())).as_deref(),
            Some("Iosevka")
        );
        assert_eq!(pick_mono(&installed, || Some("Missing".into())), None);
    }

    #[test]
    fn terminal_leads_with_a_nerd_font_and_falls_back_for_icons() {
        let installed = names(&["Menlo", "JetBrainsMono NFM", "Symbols Nerd Font Mono"]);
        let font = terminal_font(&installed, "Menlo");
        assert_eq!(font.family.as_ref(), "JetBrainsMono NFM");
        assert_eq!(
            font.fallbacks.unwrap().fallback_list(),
            ["Symbols Nerd Font Mono"]
        );

        let installed = names(&["Menlo", "MesloLGS NF", "Hack Nerd Font Mono"]);
        let font = terminal_font(&installed, "Menlo");
        assert_eq!(font.family.as_ref(), "Menlo");
        assert_eq!(
            font.fallbacks.unwrap().fallback_list(),
            ["MesloLGS NF", "Hack Nerd Font Mono"]
        );

        let font = terminal_font(&names(&["Menlo"]), "Menlo");
        assert_eq!(font.family.as_ref(), "Menlo");
        assert!(font.fallbacks.is_none());
    }
}

pub(crate) fn configured_family(requested: &str, installed: &[String], default: &str) -> String {
    if requested.is_empty() {
        return default.to_owned();
    }
    installed
        .iter()
        .find(|family| family.as_str() == requested)
        .cloned()
        .unwrap_or_else(|| default.to_owned())
}

pub(crate) fn review(cx: &App) -> (String, f32) {
    let preferences = cx.try_global::<crate::settings::Preferences>();
    let family = preferences.map_or("", |preferences| preferences.review_font_family.as_str());
    let size = preferences.map_or(12.5, |preferences| preferences.review_font_size);
    (
        if family.is_empty() {
            mono().to_owned()
        } else {
            configured_family(family, &installed(cx), mono())
        },
        crate::settings::preferences::valid_font_size(size, 12.5),
    )
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn unavailable_fonts_use_default() {
        let installed = vec!["Menlo".to_owned()];
        assert_eq!(configured_family("Menlo", &installed, "Default"), "Menlo");
        assert_eq!(
            configured_family("Missing", &installed, "Default"),
            "Default"
        );
        assert_eq!(configured_family("", &installed, "Default"), "Default");
    }
}

#[cfg(test)]
mod live_tests {
    use gpui::BorrowAppContext;
    #[gpui::test]
    fn cached_font_names_preserve_live_font_selection(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let names =
                std::sync::Arc::new(vec!["Test Mono A".to_owned(), "Test Mono B".to_owned()]);
            cx.set_global(super::InstalledFonts(std::sync::OnceLock::from(
                names.clone(),
            )));
            let mut preferences = crate::settings::Preferences::default();
            preferences.review_font_family = "Test Mono A".into();
            preferences.terminal_font_family = "Test Mono B".into();
            cx.set_global(preferences);
            for _ in 0..100 {
                assert_eq!(super::review(cx).0, "Test Mono A");
                assert_eq!(super::terminal(cx).family.as_ref(), "Test Mono B");
                assert!(std::sync::Arc::ptr_eq(&super::installed(cx), &names));
            }
            cx.update_global::<crate::settings::Preferences, _>(|preferences, _| {
                preferences.review_font_family = "Test Mono B".into();
                preferences.terminal_font_family = "Test Mono A".into();
            });
            assert_eq!(super::review(cx).0, "Test Mono B");
            assert_eq!(super::terminal(cx).family.as_ref(), "Test Mono A");
        });
    }
    #[gpui::test]
    fn terminal_and_review_preferences_are_independent(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Preferences::default());
            let default = super::terminal(cx);
            cx.update_global::<crate::settings::Preferences, _>(|preferences, _| {
                preferences.review_font_size = 24.;
                preferences.terminal_font_family = "Unavailable test font".into();
            });
            assert_eq!(super::terminal(cx).family, default.family);
            assert_eq!(super::review(cx).1, 24.);
            assert_eq!(
                cx.global::<crate::settings::Preferences>()
                    .terminal_font_size,
                13.
            );
        });
    }
}
