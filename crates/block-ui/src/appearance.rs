#![expect(
    clippy::redundant_pub_crate,
    reason = "Keep private UI modules scoped to their parent."
)]
use std::{io::Write as _, path::Path};

use gpui::{Global, WindowAppearance};
use serde::{Deserialize, Serialize};

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
}

impl Preferences {
    pub(super) fn load(root: &Path) -> anyhow::Result<Self> {
        match std::fs::read(root.join("preferences.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn save(&self, root: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(root)?;
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(root.join("preferences.json"))?;
        Ok(())
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
    pub surface: u32,
    pub foreground: u32,
    pub muted: u32,
    pub border: u32,
    pub primary: u32,
    pub on_primary: u32,
}

impl Global for Palette {}

impl Palette {
    pub(super) const LIGHT: Self = Self {
        background: 0x00ff_ffff,
        surface: 0x00f5_f5f5,
        foreground: 0x0017_1717,
        muted: 0x0073_7373,
        border: 0x00e5_e5e5,
        primary: 0x0026_2626,
        on_primary: 0x00fa_fafa,
    };
    pub(super) const DARK: Self = Self {
        background: 0x0017_1717,
        surface: 0x0026_2626,
        foreground: 0x00fa_fafa,
        muted: 0x00a3_a3a3,
        border: 0x0040_4040,
        primary: 0x00e5_e5e5,
        on_primary: 0x0026_2626,
    };
}

#[cfg(test)]
mod tests {
    use super::{Appearance, Preferences};

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
        std::fs::write(root.path().join("preferences.json"), b"invalid")?;
        assert!(Preferences::load(root.path()).is_err());
        Ok(())
    }
}
