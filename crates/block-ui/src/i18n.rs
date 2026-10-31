//! Product languages and the desktop-only bridge to component-library locale.

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

/// The supported, persistable product languages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// Simplified Chinese.
    #[serde(rename = "zh-Hans")]
    SimplifiedChinese,
    /// Japanese.
    #[serde(rename = "ja")]
    Japanese,
    /// English; unknown saved codes also resolve deterministically to English.
    #[default]
    #[serde(rename = "en", other)]
    English,
}

impl Language {
    /// Languages in selector order, named in their own languages.
    pub const ALL: [Self; 3] = [Self::English, Self::SimplifiedChinese, Self::Japanese];

    /// Canonical product locale and persisted code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-Hans",
            Self::Japanese => "ja",
        }
    }

    /// A language name that can be recognized before selecting the language.
    #[must_use]
    pub const fn self_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
            Self::Japanese => "日本語",
        }
    }

    const fn component_code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
            Self::Japanese => "ja",
        }
    }

    /// Select the first supported OS preference, falling back to English.
    #[must_use]
    pub fn from_locales<I, S>(locales: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        locales
            .into_iter()
            .find_map(|locale| match_locale(locale.as_ref()))
            .unwrap_or_default()
    }

    /// Detect the OS preference once at first startup.
    #[must_use]
    pub fn detect() -> Self {
        Self::from_locales(sys_locale::get_locales())
    }
}

fn match_locale(locale: &str) -> Option<Language> {
    let canonical = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    let mut parts = canonical.split('-');
    let language = parts.next()?;
    if language.eq_ignore_ascii_case("en") {
        return Some(Language::English);
    }
    if language.eq_ignore_ascii_case("ja") {
        return Some(Language::Japanese);
    }
    if !language.eq_ignore_ascii_case("zh") {
        return None;
    }
    let subtags: Vec<&str> = parts.collect();
    if let Some(script) = subtags
        .iter()
        .find(|part| part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
    {
        return script
            .eq_ignore_ascii_case("hans")
            .then_some(Language::SimplifiedChinese);
    }
    if subtags.is_empty()
        || subtags
            .iter()
            .any(|part| part.eq_ignore_ascii_case("cn") || part.eq_ignore_ascii_case("sg"))
    {
        Some(Language::SimplifiedChinese)
    } else {
        None
    }
}

/// Retained product copy or verbatim user/diagnostic text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UiText {
    Message {
        key: &'static str,
        args: Vec<(&'static str, String)>,
    },
    Raw(String),
}

impl UiText {
    pub(crate) const fn message(key: &'static str) -> Self {
        Self::Message {
            key,
            args: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, name: &'static str, value: impl Into<String>) -> Self {
        if let Self::Message { args, .. } = &mut self {
            args.push((name, value.into()));
        }
        self
    }

    pub(crate) const fn key(&self) -> Option<&'static str> {
        match self {
            Self::Message { key, .. } => Some(*key),
            Self::Raw(_) => None,
        }
    }

    pub(crate) fn text(&self, language: Language) -> String {
        match self {
            Self::Message { key, args } => {
                let template = tr!(language, *key);
                let names = args.iter().map(|(name, _)| *name).collect::<Vec<_>>();
                let values = args
                    .iter()
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>();
                rust_i18n::replace_patterns(&template, &names, &values)
            }
            Self::Raw(text) => text.clone(),
        }
    }
}

impl From<String> for UiText {
    fn from(text: String) -> Self {
        Self::Raw(text)
    }
}

impl From<&str> for UiText {
    fn from(text: &str) -> Self {
        Self::Raw(text.to_owned())
    }
}

pub(crate) fn profile_icon_name(
    language: Language,
    icon: block_launcher::catalog::ProfileIcon,
) -> String {
    use block_launcher::catalog::ProfileIcon;
    let key = match icon {
        ProfileIcon::Blocks => "appearance.icon.blocks",
        ProfileIcon::Layers => "appearance.icon.layers",
        ProfileIcon::Pickaxe => "appearance.icon.pickaxe",
        ProfileIcon::Sword => "appearance.icon.sword",
        ProfileIcon::Compass => "appearance.icon.compass",
        ProfileIcon::Hammer => "appearance.icon.hammer",
        ProfileIcon::Leaf => "appearance.icon.leaf",
        ProfileIcon::FlaskConical => "appearance.icon.experiment",
    };
    tr!(language, key)
}

pub(crate) fn profile_color_name(
    language: Language,
    color: block_launcher::catalog::ProfileColor,
) -> String {
    use block_launcher::catalog::ProfileColor;
    let key = match color {
        ProfileColor::Gray => "appearance.color.gray",
        ProfileColor::Blue => "appearance.color.blue",
        ProfileColor::Green => "appearance.color.green",
        ProfileColor::Orange => "appearance.color.orange",
        ProfileColor::Purple => "appearance.color.purple",
        ProfileColor::Pink => "appearance.color.pink",
    };
    tr!(language, key)
}

#[derive(Clone, Copy)]
struct DesktopLocale(Language);
impl Global for DesktopLocale {}

/// Install the one process-wide component locale bridge at the real desktop boundary.
///
/// Product views use their own explicit language; the component library uses a
/// process-wide setting. A desktop process therefore has one component locale.
pub fn initialize_desktop_locale(cx: &mut App, language: Language) {
    static EXTEND_COMPONENT: std::sync::Once = std::sync::Once::new();
    EXTEND_COMPONENT.call_once(|| rust_i18n::extend!(gpui_component));
    gpui_component::set_locale(language.component_code());
    // Activate the observer before either the initial or resolved startup
    // notification can be coalesced into the current effect cycle.
    cx.observe_global::<DesktopLocale>(|cx| {
        let language = cx.global::<DesktopLocale>().0;
        gpui_component::set_locale(language.component_code());
        cx.refresh_windows();
    })
    .detach();
    cx.set_global(DesktopLocale(language));
}

/// Notify the real desktop bridge without changing component locale in fixtures.
pub(super) fn request_component_language(cx: &mut App, language: Language) {
    if cx.has_global::<DesktopLocale>() && cx.global::<DesktopLocale>().0 != language {
        cx.set_global(DesktopLocale(language));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::Language;

    #[test]
    fn ordered_os_locales_match_supported_languages_and_scripts() {
        use Language::{English, Japanese, SimplifiedChinese};
        for (input, expected) in [
            ("en-US", English),
            ("ja_JP.UTF-8", Japanese),
            ("zh", SimplifiedChinese),
            ("zh-Hans-TW", SimplifiedChinese),
            ("zh-CN", SimplifiedChinese),
            ("zh-SG", SimplifiedChinese),
            ("zh-Hant-CN", English),
            ("zh-TW", English),
            ("zh-HK", English),
            ("zh-MO", English),
            ("fr-FR", English),
        ] {
            assert_eq!(Language::from_locales([input]), expected, "{input}");
        }
        assert_eq!(Language::from_locales(["fr", "ja-JP", "zh-CN"]), Japanese);
        assert_eq!(
            Language::from_locales(["zh-Hant", "zh-CN"]),
            SimplifiedChinese
        );
        assert_eq!(Language::from_locales(std::iter::empty::<&str>()), English);
    }

    #[test]
    fn unknown_persisted_language_is_english_not_missing() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<Language>(r#""fr""#)?,
            Language::English
        );
        assert_eq!(
            serde_json::to_string(&Language::SimplifiedChinese)?,
            r#""zh-Hans""#
        );
        Ok(())
    }

    #[test]
    fn product_lookup_is_explicitly_localized() {
        assert_eq!(tr!(Language::English, "common.close"), "Close");
        assert_eq!(tr!(Language::SimplifiedChinese, "common.close"), "关闭");
        assert_eq!(tr!(Language::Japanese, "common.close"), "閉じる");
    }

    fn placeholders(value: &str) -> BTreeSet<&str> {
        let mut remaining = value;
        let mut names = BTreeSet::new();
        while let Some((_, rest)) = remaining.split_once("%{") {
            if let Some((name, after)) = rest.split_once('}') {
                names.insert(name);
                remaining = after;
            } else {
                names.insert("");
                break;
            }
        }
        names
    }

    #[test]
    fn every_product_key_has_nonempty_messages_and_matching_placeholders() -> anyhow::Result<()> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let mut keys = BTreeSet::new();
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let document: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            let object = document
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("invalid catalog: {path:?}"))?;
            assert_eq!(
                object.get("_version").and_then(serde_json::Value::as_u64),
                Some(2)
            );
            for (key, translations) in object.iter().filter(|(key, _)| key.as_str() != "_version") {
                assert!(keys.insert(key.clone()), "duplicate key {key}");
                let translations = translations
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("invalid key {key}"))?;
                assert_eq!(translations.len(), Language::ALL.len(), "{key}");
                let mut expected = None;
                for language in Language::ALL {
                    let value = translations
                        .get(language.code())
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("{key} lacks {}", language.code()))?;
                    assert!(!value.trim().is_empty(), "{key}/{}", language.code());
                    let names = placeholders(value);
                    assert!(!names.contains(""), "malformed placeholder in {key}");
                    if let Some(expected) = &expected {
                        assert_eq!(&names, expected, "{key}/{}", language.code());
                    } else {
                        expected = Some(names);
                    }
                }
            }
        }
        assert!(!keys.is_empty());
        Ok(())
    }

    #[test]
    fn retained_message_resolves_again_without_changing_its_arguments() {
        let message = super::UiText::message("crash.session").arg("id", "run-123");
        assert_eq!(message.text(Language::English), "Session run-123");
        assert_eq!(message.text(Language::SimplifiedChinese), "会话 run-123");
        assert_eq!(message.text(Language::Japanese), "セッション run-123");
        assert_eq!(message.key(), Some("crash.session"));
    }

    #[gpui::test]
    fn desktop_bridge_updates_component_locale_in_an_isolated_process(
        cx: &mut gpui::TestAppContext,
    ) {
        const CHILD: &str = "BLOCK_I18N_BRIDGE_TEST_CHILD";
        let selected = std::env::var_os(CHILD);
        if selected.is_none() {
            for selection in ["ja", "zh-CN"] {
                let result = std::env::current_exe().and_then(|executable| {
                    std::process::Command::new(executable)
                        .arg("--exact")
                        .arg("i18n::tests::desktop_bridge_updates_component_locale_in_an_isolated_process")
                        .env(CHILD, selection)
                        .status()
                });
                assert!(result.is_ok_and(|status| status.success()), "{selection}");
            }
            return;
        }

        let (_, cx) = crate::test_support::window(cx, crate::tests::test_view);
        rust_i18n::set_locale("unsupported");
        let (language, expected) = if selected.as_deref() == Some(std::ffi::OsStr::new("ja")) {
            (Language::Japanese, "ja")
        } else {
            (Language::SimplifiedChinese, "zh-CN")
        };
        // Match bootstrap: the bridge and resolved first-start choice are set
        // in the same application update before the first normal paint.
        cx.update(|_, cx| {
            super::initialize_desktop_locale(cx, Language::English);
            super::request_component_language(cx, language);
        });
        cx.run_until_parked();
        assert_eq!(rust_i18n::locale().to_string(), expected);
        cx.update(|_, cx| super::request_component_language(cx, Language::English));
        cx.run_until_parked();
        assert_eq!(rust_i18n::locale().to_string(), "en");
    }
}
