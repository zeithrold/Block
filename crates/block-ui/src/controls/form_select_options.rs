//! Typed draft values offered by ordinary form controls.
use crate::{Language, UiAction};
pub(crate) struct Options<V> {
    pub(super) rows: Vec<(V, String)>,
    pub(super) selected: Option<V>,
    pub(super) confirm: fn(V) -> UiAction,
}
impl<V> Options<V> {
    pub(crate) fn new(
        rows: impl IntoIterator<Item = (V, String)>,
        selected: Option<V>,
        confirm: fn(V) -> UiAction,
    ) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            selected,
            confirm,
        }
    }
}
impl Options<bool> {
    pub(crate) fn runtime(
        language: Language,
        automatic: bool,
        confirm: fn(bool) -> UiAction,
    ) -> Self {
        Self::new(
            [
                (true, tr!(language, "choice.java_automatic")),
                (false, tr!(language, "choice.java_custom")),
            ],
            Some(automatic),
            confirm,
        )
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowSize {
    Preset(u32, u32),
    Custom,
}
impl Options<WindowSize> {
    pub(crate) fn window(
        language: Language,
        width: &str,
        height: &str,
        custom: bool,
        confirm: fn(WindowSize) -> UiAction,
    ) -> Self {
        let selected = if custom {
            Some(WindowSize::Custom)
        } else {
            width
                .trim()
                .parse()
                .ok()
                .zip(height.trim().parse().ok())
                .map(|(w, h)| WindowSize::Preset(w, h))
        };
        Self::new(
            crate::settings::WINDOW_PRESETS
                .into_iter()
                .map(|(w, h)| (WindowSize::Preset(w, h), format!("{w} × {h}")))
                .chain([(WindowSize::Custom, tr!(language, "choice.custom_size"))]),
            selected,
            confirm,
        )
    }
}
