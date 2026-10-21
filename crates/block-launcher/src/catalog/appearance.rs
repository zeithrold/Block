//! Persisted, bounded appearance choices; no paths, CSS or executable content.

use serde::{Deserialize, Serialize};

/// A Profile's decorative identity, independent of its installation and data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileAppearance {
    pub icon: ProfileIcon,
    pub color: ProfileColor,
}

/// Supported product icons. Unknown serialized values are rejected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileIcon {
    #[default]
    Blocks,
    Layers,
    Pickaxe,
    Sword,
    Compass,
    Hammer,
    Leaf,
    FlaskConical,
}

impl ProfileIcon {
    pub const ALL: [Self; 8] = [
        Self::Blocks,
        Self::Layers,
        Self::Pickaxe,
        Self::Sword,
        Self::Compass,
        Self::Hammer,
        Self::Leaf,
        Self::FlaskConical,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blocks => "Blocks",
            Self::Layers => "Layers",
            Self::Pickaxe => "Pickaxe",
            Self::Sword => "Sword",
            Self::Compass => "Compass",
            Self::Hammer => "Hammer",
            Self::Leaf => "Leaf",
            Self::FlaskConical => "Experiment",
        }
    }
}

/// Semantic color choices resolved by the UI's current light/dark palette.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileColor {
    #[default]
    Gray,
    Blue,
    Green,
    Orange,
    Purple,
    Pink,
}

impl ProfileColor {
    pub const ALL: [Self; 6] = [
        Self::Gray,
        Self::Blue,
        Self::Green,
        Self::Orange,
        Self::Purple,
        Self::Pink,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gray => "Gray",
            Self::Blue => "Blue",
            Self::Green => "Green",
            Self::Orange => "Orange",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
        }
    }
}
