#![expect(
    clippy::redundant_pub_crate,
    reason = "Keep private UI modules scoped to their parent."
)]
use std::path::Path;

use gpui::{Global, WindowAppearance};
use serde::{Deserialize, Serialize};

use crate::Language;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub appearance: Appearance,
    pub reduce_motion: bool,
    pub onboarding_complete: bool,
    pub language: Option<Language>,
    pub game: super::settings::GameDefaults,
    pub downloads: block_launcher::DownloadOptions,
}

impl Preferences {
    /// Read the `[preferences]` section of the global configuration document.
    ///
    /// Stored memory outside the 1–32 GB product range is clamped before
    /// validation, then persisted on the next save.
    pub(super) fn load(root: &Path) -> anyhow::Result<Self> {
        let mut preferences = match super::global_config::read(root)? {
            Some(config) => config.preferences,
            None => Self::default(),
        };
        preferences.game.memory_mib = block_launcher::clamp_memory_mib(preferences.game.memory_mib);
        preferences.validate()?;
        Ok(preferences)
    }

    /// Replace only the `[preferences]` section, preserving accounts,
    /// providers and unknown sections written by other components.
    pub(super) fn save(&self, root: &Path) -> anyhow::Result<()> {
        self.validate()?;
        let _storage = block_launcher::storage::acquire(root)?;
        super::global_config::update(root, |config| config.preferences = self.clone())
    }

    /// Detect the product language once, before the first paint.
    ///
    /// A stored choice always wins over the operating-system preference;
    /// an unknown stored code already fails closed to English in [`Language`].
    /// Only a missing choice is detected and persisted. A document that cannot
    /// be read is reported and deliberately left untouched instead of being
    /// replaced, and a failed write keeps the detected language usable for this
    /// session while returning the failure to the caller.
    pub(super) fn initialize_language(
        root: &Path,
        detected: Language,
    ) -> (Self, Option<anyhow::Error>) {
        let mut preferences = match Self::load(root) {
            Ok(preferences) => preferences,
            Err(error) => return (Self::default(), Some(error)),
        };
        if preferences.language.is_some() {
            return (preferences, None);
        }
        let (language, error) = match Self::save_missing_language(root, detected) {
            Ok(language) => (language, None),
            Err(error) => (detected, Some(error)),
        };
        preferences.language = Some(language);
        (preferences, error)
    }

    /// Persist one concrete choice without replacing any other preference.
    pub(super) fn persist_language(root: &Path, language: Language) -> anyhow::Result<()> {
        let _storage = block_launcher::storage::acquire(root)?;
        super::global_config::update(root, |config| {
            config.preferences.language = Some(language);
        })
    }

    /// Store the detected language only while the document still has no choice,
    /// so a concurrent writer's explicit selection is never overwritten.
    fn save_missing_language(root: &Path, language: Language) -> anyhow::Result<Language> {
        let _storage = block_launcher::storage::acquire(root)?;
        let mut selected = language;
        super::global_config::update(root, |config| {
            selected = *config.preferences.language.get_or_insert(language);
        })?;
        Ok(selected)
    }

    pub(super) fn validate(&self) -> anyhow::Result<()> {
        self.game.validate()?;
        self.downloads.validate()
    }

    pub(super) const fn palette(&self, system: WindowAppearance) -> Palette {
        let dark = match self.appearance {
            Appearance::System => matches!(
                system,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        if dark { Palette::DARK } else { Palette::LIGHT }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Palette {
    pub background: u32,
    /// Secondary surface, retained for existing consumers.
    pub surface: u32,
    pub foreground: u32,
    pub muted: u32,
    pub border: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub card: u32,
    pub popover: u32,
    pub secondary: u32,
    pub accent: u32,
    pub input: u32,
    pub input_background: u32,
    pub ring: u32,
    pub destructive: u32,
    pub destructive_background: u32,
    pub info: u32,
    pub info_background: u32,
    pub success: u32,
    pub success_background: u32,
    pub warning: u32,
    pub warning_background: u32,
    pub sidebar: u32,
}

impl Global for Palette {}

impl Palette {
    pub(super) const LIGHT: Self = Self {
        background: 0x00ff_ffff,
        surface: 0x00f5_f5f5,
        foreground: 0x000a_0a0a,
        muted: 0x0073_7373,
        border: 0x00e5_e5e5,
        primary: 0x0017_1717,
        on_primary: 0x00fa_fafa,
        card: 0x00ff_ffff,
        popover: 0x00ff_ffff,
        secondary: 0x00f5_f5f5,
        accent: 0x00f5_f5f5,
        input: 0x00e5_e5e5,
        input_background: 0x00ff_ffff,
        ring: 0x00a3_a3a3,
        destructive: 0x00e7_001b,
        destructive_background: 0x00fd_e6e8,
        info: 0x001d_4ed8,
        info_background: 0x00ef_f6ff,
        success: 0x0015_803d,
        success_background: 0x00f0_fdf4,
        warning: 0x00a1_6207,
        warning_background: 0x00fe_fce8,
        sidebar: 0x00f5_f5f5,
    };
    pub(super) const DARK: Self = Self {
        background: 0x0017_1717,
        surface: 0x0026_2626,
        foreground: 0x00fa_fafa,
        muted: 0x00a3_a3a3,
        border: 0x002e_2e2e,
        primary: 0x00e5_e5e5,
        on_primary: 0x0017_1717,
        card: 0x0017_1717,
        popover: 0x0017_1717,
        secondary: 0x0026_2626,
        accent: 0x0026_2626,
        input: 0x003a_3a3a,
        input_background: 0x0014_1414,
        ring: 0x0073_7373,
        destructive: 0x00ff_6467,
        destructive_background: 0x003e_2022,
        info: 0x0093_c5fd,
        info_background: 0x0017_2c4a,
        success: 0x004a_de80,
        success_background: 0x0013_2c1c,
        warning: 0x00fa_cc15,
        warning_background: 0x0034_2c12,
        sidebar: 0x0026_2626,
    };
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Appearance, Preferences};
    use crate::Language;

    #[test]
    fn preferences_survive_replacement_and_reject_corruption() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(Preferences::load(root.path())?, Preferences::default());
        let preferences = Preferences {
            appearance: Appearance::Dark,
            reduce_motion: true,
            ..Preferences::default()
        };
        preferences.save(root.path())?;
        assert_eq!(Preferences::load(root.path())?, preferences);
        Preferences::default().save(root.path())?;
        assert_eq!(Preferences::load(root.path())?, Preferences::default());
        std::fs::write(root.path().join(crate::global_config::FILE), b"invalid")?;
        assert!(Preferences::load(root.path()).is_err());
        Ok(())
    }

    #[test]
    fn legacy_settings_gain_defaults_and_invalid_drafts_preserve_saved_preferences()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join(crate::global_config::FILE);
        let original = b"[preferences]\nappearance = \"Dark\"\nreduce_motion = true\nonboarding_complete = true\n";
        std::fs::write(&path, original)?;
        let mut preferences = Preferences::load(root.path())?;
        assert_eq!(
            preferences.game,
            super::super::settings::GameDefaults::default()
        );
        assert_eq!(
            preferences.downloads,
            block_launcher::DownloadOptions::default()
        );
        assert!(preferences.onboarding_complete && preferences.reduce_motion);
        preferences.downloads.proxy = block_launcher::ProxyMode::Http {
            url: "http://user:password@localhost:8080".into(),
        };
        assert!(preferences.save(root.path()).is_err());
        assert_eq!(std::fs::read(path)?, original);
        Ok(())
    }

    #[test]
    fn loading_out_of_range_stored_memory_clamps_to_the_product_bounds() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        for (stored, expected) in [(256, 1024), (65536, 32768), (4096, 4096)] {
            let preferences = Preferences {
                onboarding_complete: true,
                game: super::super::settings::GameDefaults {
                    memory_mib: stored,
                    ..Default::default()
                },
                ..Default::default()
            };
            // Bypass validation so the out-of-range value really reaches the
            // document, exactly like a value written by an older version.
            crate::global_config::update(root.path(), |config| {
                config.preferences = preferences;
            })?;
            let loaded = Preferences::load(root.path())?;
            assert_eq!(loaded.game.memory_mib, expected);
            loaded.save(root.path())?;
            assert_eq!(Preferences::load(root.path())?.game.memory_mib, expected);
        }
        Ok(())
    }

    /// Seed one document that already holds the sections a language write must
    /// never replace: accounts, providers and an unknown top-level section.
    fn seed_foreign_sections(
        root: &Path,
    ) -> anyhow::Result<(block_auth::Account, block_auth::AuthlibProvider)> {
        std::fs::write(
            root.join(crate::global_config::FILE),
            "[other]\nkept = true\n",
        )?;
        let account = block_auth::Account {
            reference: block_auth::AccountRef {
                player_id: "00000000-0000-4000-8000-000000000001".parse()?,
            },
            provider: block_auth::AccountProvider::Offline,
            player_name: "Player".into(),
            client_id: String::new(),
            credential_ref: String::new(),
            authlib: None,
        };
        let provider = block_auth::AuthlibProvider {
            id: uuid::Uuid::new_v4(),
            name: "Community".into(),
            api_url: "https://example.invalid/api".into(),
            allow_http: false,
            metadata: block_auth::ProviderMetadata {
                server_name: None,
                icon_url: String::new(),
                skin_domains: Vec::new(),
                signature_public_key: String::new(),
            },
            metadata_json: "{}".into(),
        };
        crate::global_config::update(root, |config| {
            config.accounts = vec![account.clone()];
            config.providers = vec![provider.clone()];
        })?;
        Ok((account, provider))
    }

    #[test]
    fn first_startup_detects_and_persists_language_without_replacing_other_sections()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (account, provider) = seed_foreign_sections(root.path())?;

        let (preferences, error) =
            Preferences::initialize_language(root.path(), Language::Japanese);

        assert!(error.is_none(), "{error:?}");
        assert_eq!(preferences.language, Some(Language::Japanese));
        let document = crate::global_config::read(root.path())?.unwrap_or_default();
        assert_eq!(document.preferences.language, Some(Language::Japanese));
        assert_eq!(document.accounts, vec![account]);
        assert_eq!(document.providers, vec![provider]);
        let text = std::fs::read_to_string(root.path().join(crate::global_config::FILE))?;
        assert!(text.contains("[other]"), "{text}");
        assert!(text.contains("kept"), "{text}");
        Ok(())
    }

    #[test]
    fn stored_language_defeats_an_operating_system_change_and_is_not_rewritten()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join(crate::global_config::FILE);
        let (first, error) =
            Preferences::initialize_language(root.path(), Language::SimplifiedChinese);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(first.language, Some(Language::SimplifiedChinese));
        let written = std::fs::read(&path)?;

        let (second, error) = Preferences::initialize_language(root.path(), Language::Japanese);

        assert!(error.is_none(), "{error:?}");
        assert_eq!(second.language, Some(Language::SimplifiedChinese));
        assert_eq!(std::fs::read(&path)?, written);
        Ok(())
    }

    #[test]
    fn unknown_stored_language_is_english_and_a_corrupt_document_is_preserved() -> anyhow::Result<()>
    {
        let root = tempfile::tempdir()?;
        let path = root.path().join(crate::global_config::FILE);
        std::fs::write(&path, "[preferences]\nlanguage = \"fr\"\n")?;
        let unknown = std::fs::read(&path)?;

        let (preferences, error) =
            Preferences::initialize_language(root.path(), Language::Japanese);

        assert!(error.is_none(), "{error:?}");
        assert_eq!(preferences.language, Some(Language::English));
        assert_eq!(std::fs::read(&path)?, unknown);

        std::fs::write(&path, b"[preferences]\nlanguage = ")?;
        let corrupt = std::fs::read(&path)?;
        let (preferences, error) =
            Preferences::initialize_language(root.path(), Language::Japanese);
        assert!(error.is_some(), "a corrupt document must be reported");
        assert_eq!(preferences, Preferences::default());
        assert_eq!(std::fs::read(&path)?, corrupt);
        Ok(())
    }

    #[test]
    fn a_failed_startup_save_reports_the_error_and_keeps_the_detected_language()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        // A corrupt storage configuration rejects the data-root lease, so the
        // language write cannot start and must not touch either document.
        std::fs::write(root.path().join("block.toml"), b"not = [valid")?;

        let (preferences, error) =
            Preferences::initialize_language(root.path(), Language::Japanese);

        assert!(error.is_some(), "the unavailable lease must be reported");
        assert_eq!(preferences.language, Some(Language::Japanese));
        assert!(!root.path().join(crate::global_config::FILE).exists());
        assert_eq!(
            std::fs::read(root.path().join("block.toml"))?,
            b"not = [valid"
        );
        Ok(())
    }
}
