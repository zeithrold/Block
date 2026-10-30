//! Shared visual contracts. Values follow the actual 2026-10-21 prototype worktree.
//! Colors are resolved in appearance; resources are embedded and require no sibling checkout.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Design tokens stay inside the private UI module; only the asset source is exported."
)]

use std::borrow::Cow;

use gpui::{AssetSource, BoxShadow, Hsla, SharedString, Styled as _, Svg, point, px, rgb, svg};

pub(super) const FONT_FAMILY: &str = "Inter";

/// The same licensed Latin font as the prototype, converted to a native font
/// container once. The operating system supplies CJK and emoji fallbacks.
///
/// # Errors
/// Returns the platform font registration error if a bundled font cannot load.
pub fn load_fonts(cx: &gpui::App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
    ])
}

/// Logical pixels: these remain independent of the display scale factor.
pub(super) const CONTROL_HEIGHT: f32 = 32.;
pub(super) const CONTROL_RADIUS: f32 = 10.;
pub(super) const CARD_RADIUS: f32 = 14.;
pub(super) const FIELD_GAP: f32 = 6.;
pub(super) const BODY_FONT_SIZE: f32 = 14.;
pub(super) const SMALL_FONT_SIZE: f32 = 12.;
pub(super) const BODY_LINE_HEIGHT: f32 = 20.;
pub(super) const MANAGEMENT_FOOTER: f32 = 72.;
pub(super) const SIDEBAR_WIDTH: f32 = 360.;
pub(super) const CHROME_HEIGHT: f32 = 56.;

pub(super) fn focus_ring(color: u32) -> Vec<BoxShadow> {
    let mut color: Hsla = rgb(color).into();
    color.a = 0.5;
    vec![BoxShadow {
        inset: false,
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(3.),
    }]
}

/// Bundled monochrome Lucide paths; never rendered as font glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Check,
    Close,
    Plus,
    Minus,
    Search,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    ArrowRight,
    More,
    Menu,
    Info,
    Error,
    Warning,
    Download,
    Play,
    Stop,
    Folder,
    ExternalLink,
    Edit,
    Copy,
    Refresh,
    Settings,
    Games,
    Account,
    Sun,
    Moon,
    System,
    SignOut,
    Trash,
    Loading,
    Selector,
    Blocks,
    Layers,
    Pickaxe,
    Sword,
    Compass,
    Hammer,
    Leaf,
    Experiment,
    Grid,
    Server,
    Cpu,
    Terminal,
    Lock,
    Help,
    Home,
}

impl Icon {
    const ASSETS: &'static [(Self, &'static str, &'static [u8])] = &[
        (
            Self::Home,
            "icons/house.svg",
            include_bytes!("../../assets/icons/navigation/house.svg"),
        ),
        (
            Self::Grid,
            "icons/grid-2x2.svg",
            include_bytes!("../../assets/icons/navigation/grid-2x2.svg"),
        ),
        (
            Self::Server,
            "icons/server.svg",
            include_bytes!("../../assets/icons/navigation/server.svg"),
        ),
        (
            Self::Cpu,
            "icons/cpu.svg",
            include_bytes!("../../assets/icons/game/cpu.svg"),
        ),
        (
            Self::Terminal,
            "icons/terminal.svg",
            include_bytes!("../../assets/icons/game/terminal.svg"),
        ),
        (
            Self::Lock,
            "icons/lock-keyhole.svg",
            include_bytes!("../../assets/icons/actions/lock-keyhole.svg"),
        ),
        (
            Self::Help,
            "icons/circle-question-mark.svg",
            include_bytes!("../../assets/icons/navigation/circle-question-mark.svg"),
        ),
        (
            Self::Blocks,
            "icons/blocks.svg",
            include_bytes!("../../assets/icons/game/blocks.svg"),
        ),
        (
            Self::Layers,
            "icons/layers.svg",
            include_bytes!("../../assets/icons/game/layers.svg"),
        ),
        (
            Self::Pickaxe,
            "icons/pickaxe.svg",
            include_bytes!("../../assets/icons/game/pickaxe.svg"),
        ),
        (
            Self::Sword,
            "icons/sword.svg",
            include_bytes!("../../assets/icons/game/sword.svg"),
        ),
        (
            Self::Compass,
            "icons/compass.svg",
            include_bytes!("../../assets/icons/game/compass.svg"),
        ),
        (
            Self::Hammer,
            "icons/hammer.svg",
            include_bytes!("../../assets/icons/game/hammer.svg"),
        ),
        (
            Self::Leaf,
            "icons/leaf.svg",
            include_bytes!("../../assets/icons/game/leaf.svg"),
        ),
        (
            Self::Experiment,
            "icons/flask-conical.svg",
            include_bytes!("../../assets/icons/game/flask-conical.svg"),
        ),
        (
            Self::Check,
            "icons/check.svg",
            include_bytes!("../../assets/icons/status/check.svg"),
        ),
        (
            Self::Close,
            "icons/x.svg",
            include_bytes!("../../assets/icons/actions/x.svg"),
        ),
        (
            Self::Plus,
            "icons/plus.svg",
            include_bytes!("../../assets/icons/actions/plus.svg"),
        ),
        (
            Self::Minus,
            "icons/minus.svg",
            include_bytes!("../../assets/icons/actions/minus.svg"),
        ),
        (
            Self::Search,
            "icons/search.svg",
            include_bytes!("../../assets/icons/actions/search.svg"),
        ),
        (
            Self::ChevronLeft,
            "icons/chevron-left.svg",
            include_bytes!("../../assets/icons/controls/chevron-left.svg"),
        ),
        (
            Self::ChevronRight,
            "icons/chevron-right.svg",
            include_bytes!("../../assets/icons/controls/chevron-right.svg"),
        ),
        (
            Self::ChevronDown,
            "icons/chevron-down.svg",
            include_bytes!("../../assets/icons/controls/chevron-down.svg"),
        ),
        (
            Self::ArrowRight,
            "icons/arrow-right.svg",
            include_bytes!("../../assets/icons/controls/arrow-right.svg"),
        ),
        (
            Self::Menu,
            "icons/menu.svg",
            include_bytes!("../../assets/icons/controls/menu.svg"),
        ),
        (
            Self::More,
            "icons/ellipsis.svg",
            include_bytes!("../../assets/icons/controls/ellipsis.svg"),
        ),
        (
            Self::Info,
            "icons/info.svg",
            include_bytes!("../../assets/icons/status/info.svg"),
        ),
        (
            Self::Error,
            "icons/circle-alert.svg",
            include_bytes!("../../assets/icons/status/circle-alert.svg"),
        ),
        (
            Self::Warning,
            "icons/triangle-alert.svg",
            include_bytes!("../../assets/icons/status/triangle-alert.svg"),
        ),
        (
            Self::Download,
            "icons/download.svg",
            include_bytes!("../../assets/icons/actions/download.svg"),
        ),
        (
            Self::Play,
            "icons/play.svg",
            include_bytes!("../../assets/icons/actions/play.svg"),
        ),
        (
            Self::Stop,
            "icons/square.svg",
            include_bytes!("../../assets/icons/actions/square.svg"),
        ),
        (
            Self::Folder,
            "icons/folder.svg",
            include_bytes!("../../assets/icons/actions/folder.svg"),
        ),
        (
            Self::ExternalLink,
            "icons/external-link.svg",
            include_bytes!("../../assets/icons/actions/external-link.svg"),
        ),
        (
            Self::Edit,
            "icons/pencil.svg",
            include_bytes!("../../assets/icons/actions/pencil.svg"),
        ),
        (
            Self::Copy,
            "icons/copy.svg",
            include_bytes!("../../assets/icons/actions/copy.svg"),
        ),
        (
            Self::Refresh,
            "icons/refresh-cw.svg",
            include_bytes!("../../assets/icons/actions/refresh-cw.svg"),
        ),
        (
            Self::Settings,
            "icons/settings.svg",
            include_bytes!("../../assets/icons/navigation/settings.svg"),
        ),
        (
            Self::Games,
            "icons/gamepad-2.svg",
            include_bytes!("../../assets/icons/navigation/gamepad-2.svg"),
        ),
        (
            Self::Account,
            "icons/circle-user-round.svg",
            include_bytes!("../../assets/icons/navigation/circle-user-round.svg"),
        ),
        (
            Self::Sun,
            "icons/sun.svg",
            include_bytes!("../../assets/icons/appearance/sun.svg"),
        ),
        (
            Self::Moon,
            "icons/moon.svg",
            include_bytes!("../../assets/icons/appearance/moon.svg"),
        ),
        (
            Self::System,
            "icons/monitor.svg",
            include_bytes!("../../assets/icons/appearance/monitor.svg"),
        ),
        (
            Self::SignOut,
            "icons/log-out.svg",
            include_bytes!("../../assets/icons/actions/log-out.svg"),
        ),
        (
            Self::Trash,
            "icons/trash-2.svg",
            include_bytes!("../../assets/icons/actions/trash-2.svg"),
        ),
        (
            Self::Loading,
            "icons/loader-circle.svg",
            include_bytes!("../../assets/icons/status/loader-circle.svg"),
        ),
        (
            Self::Selector,
            "icons/chevrons-up-down.svg",
            include_bytes!("../../assets/icons/controls/chevrons-up-down.svg"),
        ),
    ];

    pub(super) fn render(self, size: f32, color: u32) -> Svg {
        svg()
            .path(self.path())
            .size(px(size))
            .flex_shrink_0()
            .text_color(rgb(color))
    }

    pub(super) fn path(self) -> &'static str {
        Self::ASSETS
            .iter()
            .find_map(|(icon, path, _)| (*icon == self).then_some(*path))
            .unwrap_or("icons/circle-alert.svg")
    }
}

/// Native asset source. The application registers this once on `Application::new()`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ASSETS
            .iter()
            .find_map(|(_, candidate, data)| (*candidate == path).then_some(Cow::Borrowed(*data))))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Icon::ASSETS
            .iter()
            .filter(|(_, candidate, _)| candidate.starts_with(path))
            .map(|(_, candidate, _)| SharedString::from(*candidate))
            .collect())
    }
}

/// Application asset source: the product's own icons plus the component
/// library's Lucide set. Native windows register exactly one asset source, so
/// the two catalogs are merged here instead of replacing each other.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiAssets;

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match Assets.load(path)? {
            Some(data) => Ok(Some(data)),
            None => gpui_kit_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        for candidate in gpui_kit_assets::Assets.list(path)? {
            if !paths.iter().any(|existing| existing == &candidate) {
                paths.push(candidate);
            }
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{Assets, Icon};
    use gpui::AssetSource as _;
    #[test]
    fn every_icon_is_shipped_without_filesystem_or_prototype_access() -> anyhow::Result<()> {
        for (icon, path, _) in Icon::ASSETS {
            assert_eq!(icon.path(), *path);
            let loaded = Assets.load(path)?;
            assert!(loaded.is_some());
            if let Some(loaded) = loaded {
                assert!(loaded.starts_with(b"<svg "));
            }
        }
        assert!(Assets.load("../outside/anything")?.is_none());
        Ok(())
    }
}
