#![expect(
    clippy::redundant_pub_crate,
    reason = "Settings are shared with the parent view and engine only."
)]
//! Validated settings drafts, with defaults applied only to newly created games.

use super::scroll_area::ScrollAreaExt as _;
use std::{cell::Cell, fmt, path::PathBuf, rc::Rc};

use anyhow::ensure;
use block_launcher::{
    DownloadOptions, ProxyMode,
    catalog::{
        DataPolicy, Profile, ProfileAppearance, ProfileInheritance, ProfileMode, ServerPreferences,
    },
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement as _,
    ParentElement as _, ScrollHandle, SharedString, Styled as _, Window, div, point, px, rgb,
};
use serde::{Deserialize, Serialize};

use super::components::{ControlState, FieldOptions};
use super::design::Icon;
use super::{
    Appearance, Block, Language, Preferences, Tab, TextInput, UiAction, components, input, note,
    patterns,
};

pub(super) const WINDOW_PRESETS: [(u32, u32); 3] = [(854, 480), (1280, 720), (1920, 1080)];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct GameDefaults {
    pub memory_mib: u32,
    pub java: String,
    pub jvm_arguments: String,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub show_snapshots: bool,
}

impl Default for GameDefaults {
    fn default() -> Self {
        Self {
            memory_mib: 2048,
            java: String::new(),
            jvm_arguments: String::new(),
            width: 1280,
            height: 720,
            fullscreen: false,
            show_snapshots: false,
        }
    }
}

impl GameDefaults {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        let mut profile = Profile {
            id: "defaults".into(),
            name: "Defaults".into(),
            instance_id: "defaults".into(),
            target_id: "vanilla".into(),
            runtime: RuntimeSelection::Managed,
            memory_mib: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            data_policy: DataPolicy::Isolated,
            default_account: None,
            compatibility: Compatibility::Official,
            appearance: ProfileAppearance::default(),
            inheritance: ProfileInheritance::default(),
            server: ServerPreferences::default(),
        };
        self.apply_to(&mut profile)
    }

    pub(super) fn apply_to(&self, profile: &mut Profile) -> anyhow::Result<()> {
        ensure!(
            (block_launcher::MIN_MEMORY_MIB..=block_launcher::MAX_MEMORY_MIB)
                .contains(&self.memory_mib),
            "Choose a memory limit between 1 and 32 GB."
        );
        ensure!(
            (320..=16384).contains(&self.width),
            "Window width must be between 320 and 16384 pixels"
        );
        ensure!(
            (240..=16384).contains(&self.height),
            "Window height must be between 240 and 16384 pixels"
        );
        ensure!(
            !self.java.contains('\0'),
            "Java path cannot contain NUL bytes"
        );
        profile.memory_mib = self.memory_mib;
        profile.runtime = if self.java.trim().is_empty() {
            RuntimeSelection::Managed
        } else {
            RuntimeSelection::Local {
                java: PathBuf::from(self.java.trim()),
            }
        };
        profile.jvm_args = super::parse_arguments(&self.jvm_arguments)?;
        profile.game_args = vec![
            "--width".into(),
            self.width.to_string(),
            "--height".into(),
            self.height.to_string(),
        ];
        if self.fullscreen {
            profile.game_args.push("--fullscreen".into());
        }
        profile.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsField {
    Memory,
    Java,
    Jvm,
    Width,
    Height,
    Concurrent,
    Retries,
    Rate,
    ProxyUrl,
}

impl SettingsField {
    const fn page(self) -> u8 {
        match self {
            Self::Memory | Self::Java | Self::Jvm | Self::Width | Self::Height => 1,
            Self::Concurrent | Self::Retries | Self::Rate | Self::ProxyUrl => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SettingsError {
    pub field: Option<SettingsField>,
    message: SettingsMessage,
}

/// Retained settings feedback is stored as product identity and translated
/// only when it is rendered, so a later language switch never leaves an older
/// language frozen in the draft.
#[derive(Clone, Debug)]
enum SettingsMessage {
    /// Product-authored message with no interpolation.
    Product(&'static str),
    /// Product-authored message carrying one diagnostic argument.
    Detail(&'static str, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsNotice {
    Saved,
    DefaultsRestored,
}

impl SettingsNotice {
    const fn key(self) -> &'static str {
        match self {
            Self::Saved => "settings.notice.saved",
            Self::DefaultsRestored => "settings.notice.defaults_restored",
        }
    }
}

impl SettingsError {
    fn at(field: SettingsField, key: &'static str) -> Self {
        Self {
            field: Some(field),
            message: SettingsMessage::Product(key),
        }
    }

    fn detail(field: SettingsField, key: &'static str, detail: impl fmt::Display) -> Self {
        Self {
            field: Some(field),
            message: SettingsMessage::Detail(key, detail.to_string()),
        }
    }

    fn save(detail: String) -> Self {
        Self {
            field: None,
            message: SettingsMessage::Detail("settings.error.save", detail),
        }
    }

    pub(super) fn message(&self, language: Language) -> String {
        self.text().text(language)
    }

    fn text(&self) -> crate::i18n::UiText {
        match &self.message {
            SettingsMessage::Product(key) => msg!(key),
            SettingsMessage::Detail(key, detail) => msg!(key, detail = detail),
        }
    }
}

/// Effective on-disk locations shown by the read-only Storage page.
#[derive(Clone, Debug)]
pub(super) struct StorageState {
    pub paths: Result<StoragePaths, StorageProblem>,
}

/// Why the Storage page cannot show effective locations.
#[derive(Clone, Debug)]
pub(super) enum StorageProblem {
    /// Block has not reached the point where its data root is available.
    Unavailable,
    /// A storage document owned outside the settings view is unreadable.
    Unreadable(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StoragePaths {
    pub library: PathBuf,
    pub cache: PathBuf,
}

impl StorageProblem {
    fn message(&self, language: Language) -> String {
        match self {
            Self::Unavailable => tr!(language, "settings.storage.unavailable"),
            Self::Unreadable(detail) => tr!(
                language,
                "settings.storage.unreadable",
                detail = detail.as_str()
            ),
        }
    }
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            paths: Ok(StoragePaths::default()),
        }
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent preferences and retained custom editor modes can coexist."
)]
pub(super) struct SettingsDraft {
    pub memory: Entity<TextInput>,
    pub java: Entity<TextInput>,
    pub java_automatic: bool,
    pub window_custom: bool,
    pub jvm: Entity<TextInput>,
    pub width: Entity<TextInput>,
    pub height: Entity<TextInput>,
    pub concurrent: Entity<TextInput>,
    pub retries: Entity<TextInput>,
    pub rate: Entity<TextInput>,
    pub proxy_url: Entity<TextInput>,
    pub proxy_mode: u8,
    pub fullscreen: bool,
    pub show_snapshots: bool,
    pub appearance: Appearance,
    pub reduce_motion: bool,
    pub page: u8,
    pub error: Option<SettingsError>,
    pub notice: Option<SettingsNotice>,
    heading: FocusHandle,
    scroll: ScrollHandle,
    navigation_scroll: ScrollHandle,
    reveal: Rc<Cell<Option<SettingsField>>>,
}

impl fmt::Debug for SettingsDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsDraft")
            .field("page", &self.page)
            .field("appearance", &self.appearance)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl SettingsDraft {
    pub(super) fn refresh_placeholders(&self, language: Language, cx: &mut Context<'_, Block>) {
        for (input, key) in [
            (&self.java, "settings.placeholder.java"),
            (&self.jvm, "settings.placeholder.jvm"),
            (&self.rate, "settings.placeholder.rate"),
        ] {
            let placeholder = tr!(language, key);
            input.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
        }
    }

    /// Independent display state for an inert outgoing transition. Inputs and
    /// scroll/focus handles are copied; live draft observers are not attached.
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        let navigation_scroll = ScrollHandle::new();
        navigation_scroll.set_offset(self.navigation_scroll.offset());
        Self {
            memory: super::frozen_input(&self.memory, cx),
            java: super::frozen_input(&self.java, cx),
            java_automatic: self.java_automatic,
            window_custom: self.window_custom,
            jvm: super::frozen_input(&self.jvm, cx),
            width: super::frozen_input(&self.width, cx),
            height: super::frozen_input(&self.height, cx),
            concurrent: super::frozen_input(&self.concurrent, cx),
            retries: super::frozen_input(&self.retries, cx),
            rate: super::frozen_input(&self.rate, cx),
            proxy_url: super::frozen_input(&self.proxy_url, cx),
            proxy_mode: self.proxy_mode,
            fullscreen: self.fullscreen,
            show_snapshots: self.show_snapshots,
            appearance: self.appearance,
            reduce_motion: self.reduce_motion,
            page: self.page,
            error: self.error.clone(),
            notice: self.notice,
            heading: cx.focus_handle(),
            scroll,
            navigation_scroll,
            reveal: Rc::new(Cell::new(self.reveal.get())),
        }
    }

    pub(super) fn new(preferences: &Preferences, cx: &mut Context<'_, Block>) -> Self {
        let game = &preferences.game;
        let downloads = &preferences.downloads;
        let language = preferences.language.unwrap_or_default();
        let (proxy_mode, proxy_url) = match &downloads.proxy {
            ProxyMode::System => (0, ""),
            ProxyMode::Direct => (1, ""),
            ProxyMode::Http { url } | ProxyMode::Socks5 { url } => (2, url.as_str()),
        };
        let scroll = ScrollHandle::new();
        let draft = Self {
            memory: input(&components::memory_gb_text(game.memory_mib), "2", cx),
            java: input(&game.java, &tr!(language, "settings.placeholder.java"), cx),
            java_automatic: game.java.is_empty(),
            window_custom: !WINDOW_PRESETS.contains(&(game.width, game.height)),
            jvm: input(
                &game.jvm_arguments,
                &tr!(language, "settings.placeholder.jvm"),
                cx,
            ),
            width: input(&game.width.to_string(), "1280", cx),
            height: input(&game.height.to_string(), "720", cx),
            concurrent: input(&downloads.max_concurrent.to_string(), "8", cx),
            retries: input(&downloads.max_retries.to_string(), "2", cx),
            rate: input(
                &format_rate(downloads.bytes_per_second),
                &tr!(language, "settings.placeholder.rate"),
                cx,
            ),
            proxy_url: input(proxy_url, "http://127.0.0.1:8080", cx),
            proxy_mode,
            fullscreen: game.fullscreen,
            show_snapshots: game.show_snapshots,
            appearance: preferences.appearance,
            reduce_motion: preferences.reduce_motion,
            page: 0,
            error: None,
            notice: None,
            heading: cx.focus_handle(),
            reveal: Rc::new(Cell::new(None)),
            scroll,
            navigation_scroll: ScrollHandle::new(),
        };
        draft.observe_fields(cx);
        draft
    }

    fn observe_fields(&self, cx: &mut Context<'_, Block>) {
        // TextInput owns IME editing, while its parent owns dirty/save feedback.
        // Observing the actual entity keeps that feedback live for typing/paste/undo.
        for field in [
            SettingsField::Memory,
            SettingsField::Java,
            SettingsField::Jvm,
            SettingsField::Width,
            SettingsField::Height,
            SettingsField::Concurrent,
            SettingsField::Retries,
            SettingsField::Rate,
            SettingsField::ProxyUrl,
        ] {
            cx.observe(self.input(field), move |view, observed, cx| {
                if let Some(draft) = &mut view.settings_form
                    && draft.input(field).entity_id() == observed.entity_id()
                    && draft
                        .error
                        .as_ref()
                        .is_some_and(|error| error.field == Some(field))
                {
                    draft.error = draft
                        .parse(&view.saved_preferences, cx)
                        .err()
                        .filter(|error| error.field == Some(field));
                }
                cx.notify();
            })
            .detach();
        }
    }

    const fn input(&self, field: SettingsField) -> &Entity<TextInput> {
        match field {
            SettingsField::Memory => &self.memory,
            SettingsField::Java => &self.java,
            SettingsField::Jvm => &self.jvm,
            SettingsField::Width => &self.width,
            SettingsField::Height => &self.height,
            SettingsField::Concurrent => &self.concurrent,
            SettingsField::Retries => &self.retries,
            SettingsField::Rate => &self.rate,
            SettingsField::ProxyUrl => &self.proxy_url,
        }
    }

    pub(super) fn fields(&self) -> Vec<Entity<TextInput>> {
        match self.page {
            1 => {
                let mut fields = vec![self.memory.clone()];
                if !self.java_automatic {
                    fields.push(self.java.clone());
                }
                if self.window_custom {
                    fields.extend([self.width.clone(), self.height.clone()]);
                }
                fields.push(self.jvm.clone());
                fields
            }
            3 => {
                let mut fields = vec![
                    self.concurrent.clone(),
                    self.retries.clone(),
                    self.rate.clone(),
                ];
                if self.proxy_mode == 2 {
                    fields.push(self.proxy_url.clone());
                }
                fields
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn is_dirty(&self, saved: &Preferences, cx: &gpui::App) -> bool {
        !self.parse(saved, cx).is_ok_and(|value| value == *saved)
    }

    pub(super) fn parse(
        &self,
        saved: &Preferences,
        cx: &gpui::App,
    ) -> Result<Preferences, SettingsError> {
        let mut preferences = saved.clone();
        preferences.appearance = self.appearance;
        preferences.reduce_motion = self.reduce_motion;
        preferences.game = self.parse_game(cx)?;
        preferences.downloads = self.parse_downloads(cx)?;
        Ok(preferences)
    }

    fn parse_game(&self, cx: &gpui::App) -> Result<GameDefaults, SettingsError> {
        let java = if self.java_automatic {
            String::new()
        } else {
            let path = self.java.read(cx).value().trim().to_owned();
            if path.is_empty() {
                return Err(SettingsError::at(
                    SettingsField::Java,
                    "settings.error.java_path",
                ));
            }
            path
        };
        let memory_mib = components::parse_memory_gb(&self.memory.read(cx).value())
            .map_err(|error| SettingsError::at(SettingsField::Memory, error.key()))?;
        let game = GameDefaults {
            memory_mib,
            java,
            jvm_arguments: self.jvm.read(cx).value(),
            width: number(
                &self.width,
                SettingsField::Width,
                "settings.error.width_number",
                cx,
            )?,
            height: number(
                &self.height,
                SettingsField::Height,
                "settings.error.height_number",
                cx,
            )?,
            fullscreen: self.fullscreen,
            show_snapshots: self.show_snapshots,
        };
        for (valid, field, key) in [
            (
                (320..=16384).contains(&game.width),
                SettingsField::Width,
                "settings.error.width_range",
            ),
            (
                (240..=16384).contains(&game.height),
                SettingsField::Height,
                "settings.error.height_range",
            ),
        ] {
            if !valid {
                return Err(SettingsError::at(field, key));
            }
        }
        GameDefaults {
            java: game.java.clone(),
            ..GameDefaults::default()
        }
        .validate()
        .map_err(|error| {
            SettingsError::detail(SettingsField::Java, "settings.error.java", error)
        })?;
        super::parse_arguments(&game.jvm_arguments).map_err(|_| {
            SettingsError::at(SettingsField::Jvm, "common.arguments_unclosed_quote")
        })?;
        game.validate().map_err(|error| {
            SettingsError::detail(SettingsField::Jvm, "settings.error.jvm", error)
        })?;
        Ok(game)
    }

    pub(super) fn parse_downloads(&self, cx: &gpui::App) -> Result<DownloadOptions, SettingsError> {
        let concurrent = number(
            &self.concurrent,
            SettingsField::Concurrent,
            "settings.error.concurrent_number",
            cx,
        )?;
        let retries = number(
            &self.retries,
            SettingsField::Retries,
            "settings.error.retries_number",
            cx,
        )?;
        if !(1..=16).contains(&concurrent) {
            return Err(SettingsError::at(
                SettingsField::Concurrent,
                "settings.error.concurrent_range",
            ));
        }
        if retries > 10 {
            return Err(SettingsError::at(
                SettingsField::Retries,
                "settings.error.retries_range",
            ));
        }
        let rate = parse_rate(&self.rate.read(cx).value())
            .map_err(|error| SettingsError::at(SettingsField::Rate, error.key()))?;
        let proxy = match self.proxy_mode {
            0 => ProxyMode::System,
            1 => ProxyMode::Direct,
            2 => ProxyMode::manual(self.proxy_url.read(cx).value().trim().into()).map_err(
                |error| {
                    SettingsError::detail(SettingsField::ProxyUrl, "settings.error.proxy", error)
                },
            )?,
            _ => {
                return Err(SettingsError::at(
                    SettingsField::ProxyUrl,
                    "settings.error.proxy_mode",
                ));
            }
        };
        let downloads = DownloadOptions {
            max_concurrent: concurrent,
            max_retries: retries,
            bytes_per_second: rate,
            proxy,
        };
        downloads.validate().map_err(|error| {
            SettingsError::detail(SettingsField::ProxyUrl, "settings.error.downloads", error)
        })?;
        Ok(downloads)
    }

    fn restore_page(&mut self, cx: &mut Context<'_, Block>) {
        let defaults = Preferences::default();
        match self.page {
            0 => {
                self.appearance = defaults.appearance;
                self.reduce_motion = defaults.reduce_motion;
            }
            1 => {
                for (input, value) in [
                    (
                        &self.memory,
                        components::memory_gb_text(defaults.game.memory_mib),
                    ),
                    (&self.java, defaults.game.java),
                    (&self.jvm, defaults.game.jvm_arguments),
                    (&self.width, defaults.game.width.to_string()),
                    (&self.height, defaults.game.height.to_string()),
                ] {
                    input.update(cx, |input, cx| input.set_value(&value, cx));
                }
                self.fullscreen = defaults.game.fullscreen;
                self.show_snapshots = defaults.game.show_snapshots;
                self.java_automatic = true;
                self.window_custom = false;
            }
            3 => {
                for (input, value) in [
                    (
                        &self.concurrent,
                        defaults.downloads.max_concurrent.to_string(),
                    ),
                    (&self.retries, defaults.downloads.max_retries.to_string()),
                    (&self.rate, String::new()),
                    (&self.proxy_url, String::new()),
                ] {
                    input.update(cx, |input, cx| input.set_value(&value, cx));
                }
                self.proxy_mode = 0;
            }
            _ => return,
        }
        self.error = None;
        self.notice = Some(SettingsNotice::DefaultsRestored);
    }
}

fn number<T: std::str::FromStr>(
    input: &Entity<TextInput>,
    field: SettingsField,
    key: &'static str,
    cx: &gpui::App,
) -> Result<T, SettingsError> {
    input
        .read(cx)
        .value()
        .trim()
        .parse()
        .map_err(|_| SettingsError::at(field, key))
}

/// Why an editable download rate cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RateError {
    Format,
    TooLarge,
}

impl RateError {
    const fn key(self) -> &'static str {
        match self {
            Self::Format => "settings.error.rate_format",
            Self::TooLarge => "settings.error.rate_large",
        }
    }
}

impl fmt::Display for RateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&tr!(Language::English, self.key()))
    }
}

impl std::error::Error for RateError {}

/// Decimal MB/s, with at most six fractional digits, maps exactly to whole bytes.
fn parse_rate(value: &str) -> Result<Option<u64>, RateError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
    {
        return Err(RateError::Format);
    }
    let whole: u64 = whole.parse().map_err(|_| RateError::TooLarge)?;
    let fractional: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().map_err(|_| RateError::TooLarge)?
    };
    let scale = 10u64
        .checked_pow(u32::try_from(6 - fraction.len()).map_err(|_| RateError::TooLarge)?)
        .ok_or(RateError::TooLarge)?;
    let bytes = whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fractional * scale))
        .ok_or(RateError::TooLarge)?;
    Ok((bytes != 0).then_some(bytes))
}

fn format_rate(value: Option<u64>) -> String {
    value.map_or_else(String::new, |bytes| {
        let whole = bytes / 1_000_000;
        let fraction = bytes % 1_000_000;
        if fraction == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{fraction:06}")
                .trim_end_matches('0')
                .to_owned()
        }
    })
}

impl Block {
    pub(super) fn set_default_java_automatic(
        &mut self,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(draft) = &mut self.settings_form {
            draft.java_automatic = automatic;
            if draft
                .error
                .as_ref()
                .is_some_and(|error| error.field == Some(SettingsField::Java))
            {
                draft.error = None;
            }
            if !automatic {
                draft.java.read(cx).focus_handle(cx).focus(window, cx);
                draft.reveal.set(Some(SettingsField::Java));
            }
        }
        cx.notify();
    }

    pub(super) fn set_default_window(
        &mut self,
        width: u32,
        height: u32,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(draft) = &mut self.settings_form {
            draft
                .width
                .update(cx, |input, cx| input.set_value(&width.to_string(), cx));
            draft
                .height
                .update(cx, |input, cx| input.set_value(&height.to_string(), cx));
            draft.window_custom = false;
            if draft.error.as_ref().is_some_and(|error| {
                matches!(
                    error.field,
                    Some(SettingsField::Width | SettingsField::Height)
                )
            }) {
                draft.error = None;
            }
        }
        cx.notify();
    }

    pub(super) fn custom_default_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(draft) = &mut self.settings_form {
            draft.window_custom = true;
            draft.width.read(cx).focus_handle(cx).focus(window, cx);
            draft.reveal.set(Some(SettingsField::Width));
        }
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, cx: &mut Context<'_, Self>) {
        self.shell.storage.paths = self.engine.as_ref().map_or_else(
            || Err(StorageProblem::Unavailable),
            |engine| {
                block_launcher::storage::Locations::load(&engine.root)
                    .and_then(|locations| {
                        let validated = locations.validate(&engine.root)?;
                        Ok(StoragePaths {
                            library: validated.library(&engine.root),
                            cache: validated.cache(&engine.root),
                        })
                    })
                    .map_err(|error| StorageProblem::Unreadable(format!("{error:#}")))
            },
        );
        let mut draft = SettingsDraft::new(&self.saved_preferences, cx);
        draft.page = self.shell.settings_page.min(4);
        self.settings_form = Some(draft);
    }

    pub(super) fn preview_appearance(&mut self, appearance: Appearance) {
        if let Some(draft) = &mut self.settings_form {
            draft.appearance = appearance;
            draft.error = None;
        }
        self.preferences.appearance = appearance;
    }

    pub(super) fn preview_reduce_motion(&mut self) {
        if let Some(draft) = &mut self.settings_form {
            draft.reduce_motion = !draft.reduce_motion;
            draft.error = None;
            self.preferences.reduce_motion = draft.reduce_motion;
        }
    }

    pub(super) fn select_settings_page(
        &mut self,
        page: u8,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.shell.settings_page = page.min(4);
        if let Some(draft) = &mut self.settings_form {
            draft.page = self.shell.settings_page;
            draft.error = None;
            draft.scroll.set_offset(point(px(0.), px(0.)));
            draft.heading.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn reset_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(draft) = &mut self.settings_form else {
            return;
        };
        draft.restore_page(cx);
        let page = draft.page;
        self.preferences.appearance = draft.appearance;
        self.preferences.reduce_motion = draft.reduce_motion;
        if page == 0 {
            // Restoring General selects a concrete English choice instead of
            // re-running operating-system detection on the next start.
            self.preferences.language = Some(Language::English);
        }
        self.refresh_language(window, cx);
        cx.notify();
    }

    pub(super) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let parsed = self.settings_form.as_ref().map(|draft| {
            draft
                .parse(&self.saved_preferences, cx)
                .map(|mut preferences| {
                    // The language is a live preview owned by the view, not by a
                    // draft input, so it is committed from the current preview.
                    preferences.language = self.preferences.language;
                    preferences
                })
        });
        let Some(parsed) = parsed else {
            return;
        };
        match parsed {
            Ok(preferences) => {
                let result = self
                    .engine
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Block's data directory is unavailable"))
                    .and_then(|engine| preferences.save(&engine.root));
                match result {
                    Ok(()) => {
                        self.preferences.clone_from(&preferences);
                        self.saved_preferences = preferences;
                        if let Some(draft) = &mut self.settings_form {
                            draft.error = None;
                            draft.notice = Some(SettingsNotice::Saved);
                        }
                        self.status = msg!("settings.status.saved");
                        if let Some(engine) = &self.engine {
                            engine.load_versions();
                        }
                    }
                    Err(error) => {
                        self.settings_failure(SettingsError::save(format!("{error:#}")), window, cx)
                    }
                }
            }
            Err(error) => self.settings_failure(error, window, cx),
        }
    }

    pub(super) fn settings_failure(
        &mut self,
        error: SettingsError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.report(error.text());
        if let Some(draft) = &mut self.settings_form {
            if let Some(field) = error.field {
                if matches!(field, SettingsField::Width | SettingsField::Height) {
                    draft.window_custom = true;
                }
                if field == SettingsField::Java {
                    draft.java_automatic = false;
                }
                draft.page = field.page();
                self.shell.settings_page = draft.page;
                draft
                    .input(field)
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                draft.reveal.set(Some(field));
            } else {
                draft.heading.focus(window, cx);
            }
            draft.notice = None;
            draft.error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        cx: &Context<'_, Self>,
    ) -> super::page_host::ManagementSlots {
        let Some(draft) = &self.settings_form else {
            return super::page_host::ManagementSlots {
                sidebar: div().into_any_element(),
                detail: div().into_any_element(),
            };
        };
        let navigation = self.settings_navigation(draft, cx);
        super::page_host::ManagementSlots {
            sidebar: navigation.into_any_element(),
            detail: self.render_settings_detail(cx),
        }
    }

    pub(super) fn render_settings_detail(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        self.settings_form.as_ref().map_or_else(
            || div().into_any_element(),
            |draft| self.settings_detail(draft, cx).into_any_element(),
        )
    }

    fn settings_navigation(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let page = draft.page;
        let language = self.language();
        let dirty = self.settings_dirty(draft, cx);
        let categories = [
            (
                0,
                "general",
                tr!(language, "settings.navigation.general"),
                Icon::System,
            ),
            (
                1,
                "game",
                tr!(language, "settings.navigation.game"),
                Icon::Settings,
            ),
            (
                2,
                "profiles",
                tr!(language, "settings.navigation.profiles"),
                Icon::Games,
            ),
            (
                3,
                "downloads",
                tr!(language, "settings.navigation.downloads"),
                Icon::Download,
            ),
            (
                4,
                "storage",
                tr!(language, "settings.navigation.storage"),
                Icon::Folder,
            ),
        ];
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("settings-categories")
                    .flex_1()
                    .min_h_0()
                    .scroll_area(
                        &draft.navigation_scroll,
                        "settings.categories",
                        self.palette,
                        false,
                    )
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(categories.map(|(index, id, label, icon)| {
                        let active = page == index;
                        let mut button = components::button(
                            format!("settings-page-{id}"),
                            "",
                            self.palette,
                            components::ButtonVariant::Ghost,
                            ControlState::default(),
                            cx.listener(move |view, (): &(), window, cx| {
                                view.handle(UiAction::SettingsPage(index), window, cx);
                            }),
                        )
                        .w_full()
                        .h(px(40.))
                        .gap_3()
                        .justify_start()
                        .child(icon.render(
                            16.,
                            if active {
                                self.palette.foreground
                            } else {
                                self.palette.muted
                            },
                        ))
                        .child(label);
                        if active {
                            button = button
                                .bg(rgb(self.palette.accent))
                                .text_color(rgb(self.palette.foreground));
                        }
                        button
                    })),
            )
            .child(
                patterns::management_footer(self.palette)
                    .justify_center()
                    .child(note(if dirty {
                        tr!(language, "settings.navigation.unsaved")
                    } else {
                        tr!(language, "settings.navigation.saved")
                    })),
            )
    }

    /// The language preview lives in the view rather than a draft input, so a
    /// language-only change still enables Save and is reported as unsaved.
    fn settings_dirty(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> bool {
        self.preferences.language != self.saved_preferences.language
            || draft.is_dirty(&self.saved_preferences, cx)
    }

    fn settings_detail(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let page = draft.page;
        let language = self.language();
        let (title, description) = match page {
            1 => (
                tr!(language, "settings.page.game.title"),
                tr!(language, "settings.page.game.description"),
            ),
            2 => (
                tr!(language, "settings.page.profiles.title"),
                tr!(language, "settings.page.profiles.description"),
            ),
            3 => (
                tr!(language, "settings.page.downloads.title"),
                tr!(language, "settings.page.downloads.description"),
            ),
            4 => (
                tr!(language, "settings.page.storage.title"),
                tr!(language, "settings.page.storage.description"),
            ),
            _ => (
                tr!(language, "settings.page.general.title"),
                tr!(language, "settings.page.general.description"),
            ),
        };
        let feedback = draft
            .error
            .as_ref()
            .map(|error| (error.message(language), true))
            .or_else(|| {
                draft
                    .notice
                    .map(|notice| (tr!(language, notice.key()), false))
            });
        let body = self.settings_section_body(draft, cx);
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .scroll_area(&draft.scroll, "settings.fields", self.palette, false)
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(608.))
                            .p_8()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(
                                patterns::page_heading(title, description)
                                    .track_focus(&draft.heading),
                            )
                            .children(feedback.map(|(message, error)| {
                                note(message)
                                    .id("settings-status")
                                    .debug_selector(|| "settings-status".into())
                                    .text_color(rgb(if error {
                                        self.palette.destructive
                                    } else {
                                        self.palette.success
                                    }))
                            }))
                            .child(body),
                    ),
            )
            .child(self.settings_footer(draft, cx))
    }

    fn settings_section_body(
        &self,
        draft: &SettingsDraft,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        match draft.page {
            1 => self.settings_fields(draft, cx),
            2 => self.settings_profile_mode(cx),
            3 => self.settings_downloads(draft, cx),
            4 => self.settings_storage(),
            _ => self.settings_appearance(draft, cx),
        }
        .into_any_element()
    }

    fn settings_footer(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let page = draft.page;
        let language = self.language();
        let dirty = self.settings_dirty(draft, cx);
        let close = UiAction::Tab(Tab::Home);
        let mut footer = patterns::management_footer(self.palette).justify_between();
        if page == 4 {
            return footer
                .child(note(tr!(language, "settings.storage.footer")).flex_1())
                .child(self.button(
                    "cancel-preferences",
                    tr!(language, "common.close"),
                    close,
                    cx,
                ));
        }
        if page == 2 {
            footer = footer.child(note(tr!(language, "settings.profile_mode.footer")).flex_1());
        } else {
            footer = footer.child(self.button(
                "reset-preferences",
                tr!(language, "settings.restore_defaults"),
                UiAction::ResetPreferences,
                cx,
            ));
        }
        footer = footer.child(
            div()
                .flex()
                .gap_2()
                .child(self.button(
                    "cancel-preferences",
                    tr!(language, "common.cancel"),
                    close,
                    cx,
                ))
                .child(components::button(
                    "save-preferences",
                    tr!(language, "settings.save_changes"),
                    self.palette,
                    components::ButtonVariant::Primary,
                    ControlState {
                        disabled: !dirty,
                        busy: false,
                        invalid: false,
                    },
                    cx.listener(|view, (): &(), window, cx| {
                        view.handle(UiAction::SavePreferences, window, cx);
                    }),
                )),
        );
        footer
    }

    fn settings_control(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        checked: bool,
        action: UiAction,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::checkbox(
            id,
            label,
            checked,
            self.palette,
            ControlState::default(),
            cx.listener(move |view, (): &(), window, cx| view.handle(action.clone(), window, cx)),
        )
    }

    fn settings_field(
        &self,
        draft: &SettingsDraft,
        field: SettingsField,
        label: impl Into<SharedString>,
        hint: Option<SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        let language = self.language();
        let error = draft
            .error
            .as_ref()
            .filter(|error| error.field == Some(field))
            .map(|error| error.message(language).into());
        patterns::reveal_field(
            components::field_with(
                label,
                draft.input(field),
                self.palette,
                FieldOptions {
                    hint,
                    error,
                    required: false,
                },
            ),
            draft.scroll.clone(),
            draft.reveal.clone(),
            field,
        )
        .id(SharedString::from(format!("settings-field-{field:?}")))
        .debug_selector(move || format!("settings-field-{field:?}"))
    }

    fn adjust_settings_memory(&mut self, increase: bool, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(draft) = &mut self.settings_form else {
            return;
        };
        let memory = draft.memory.read(cx);
        if memory.is_composing() {
            return;
        }
        let Ok(value) = components::parse_memory_gb(&memory.value()) else {
            return;
        };
        let next = if increase {
            value
                .saturating_add(1024)
                .min(block_launcher::MAX_MEMORY_MIB)
        } else {
            value
                .saturating_sub(1024)
                .max(block_launcher::MIN_MEMORY_MIB)
        };
        draft.memory.update(cx, |input, cx| {
            input.set_value(&components::memory_gb_text(next), cx);
        });
        draft.notice = None;
        cx.notify();
    }

    fn settings_memory_field(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let value = components::parse_memory_gb(&draft.memory.read(cx).value()).ok();
        let composing = draft.memory.read(cx).is_composing();
        let friendly = value.map_or_else(
            || tr!(language, "settings.game.memory.invalid"),
            components::memory_label,
        );

        let step = |increase: bool| {
            components::button_sized(
                if increase {
                    "settings-memory-increase"
                } else {
                    "settings-memory-decrease"
                },
                "",
                self.palette,
                components::ButtonVariant::Outline,
                components::ButtonSize::Icon,
                ControlState {
                    disabled: self.busy
                        || composing
                        || value.is_none_or(|value| {
                            if increase {
                                value == block_launcher::MAX_MEMORY_MIB
                            } else {
                                value == block_launcher::MIN_MEMORY_MIB
                            }
                        }),
                    ..Default::default()
                },
                cx.listener(move |view, (): &(), _, cx| view.adjust_settings_memory(increase, cx)),
            )
            .child(
                if increase { Icon::Plus } else { Icon::Minus }
                    .render(14., self.palette.foreground),
            )
        };
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.settings_field(
                draft,
                SettingsField::Memory,
                tr!(language, "settings.game.memory.label"),
                Some(tr!(language, "settings.game.memory.hint").into()),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(step(false))
                    .child(step(true))
                    .child(note(friendly).id("settings-memory-friendly")),
            )
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers == gpui::Modifiers::none()
                    && matches!(event.keystroke.key.as_str(), "up" | "down")
                    && view.settings_form.as_ref().is_some_and(|draft| {
                        !draft.memory.read(cx).is_composing()
                            && draft.memory.read(cx).focus_handle(cx).is_focused(window)
                    })
                {
                    view.adjust_settings_memory(event.keystroke.key == "up", cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn settings_appearance(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let appearance = match draft.appearance {
            Appearance::System => tr!(language, "settings.appearance.system"),
            Appearance::Light => tr!(language, "settings.appearance.light"),
            Appearance::Dark => tr!(language, "settings.appearance.dark"),
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.settings_language(cx))
            .child(self.product_select_field(
                "settings-appearance",
                tr!(language, "select.appearance"),
                &appearance,
                super::select::ProductChoice::Appearance,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.settings_control(
                        "reduce-motion",
                        tr!(language, "settings.reduce_motion.label"),
                        draft.reduce_motion,
                        UiAction::ReduceMotion,
                        cx,
                    ))
                    .child(note(tr!(language, "settings.reduce_motion.hint"))),
            )
            .child(note(tr!(language, "settings.preview.hint")))
    }

    /// The live language preview on the General page.
    ///
    fn settings_language(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.product_select_field(
                "settings-language",
                tr!(language, "settings.language.label"),
                language.self_name(),
                super::select::ProductChoice::Language,
                cx,
            ))
            .child(note(tr!(language, "settings.language.hint")))
    }

    fn settings_fields(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let java = if draft.java_automatic {
            tr!(language, "settings.game.java.automatic")
        } else {
            tr!(language, "settings.game.java.custom")
        };
        let width = draft.width.read(cx).value();
        let height = draft.height.read(cx).value();
        let window = if draft.window_custom {
            tr!(
                language,
                "settings.game.window.custom",
                width = width.as_str(),
                height = height.as_str()
            )
        } else {
            tr!(
                language,
                "settings.game.window.size",
                width = width.as_str(),
                height = height.as_str()
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                note(tr!(language, "settings.game.note"))
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(self.palette.secondary)),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(self.settings_memory_field(draft, cx).flex_1())
                    .child(
                        self.product_select_field(
                            "settings-default-java",
                            tr!(language, "select.default_java"),
                            &java,
                            super::select::ProductChoice::DefaultJava,
                            cx,
                        )
                        .flex_1(),
                    ),
            )
            .children((!draft.java_automatic).then(|| {
                self.settings_field(
                    draft,
                    SettingsField::Java,
                    tr!(language, "settings.game.java.path.label"),
                    Some(tr!(language, "settings.game.java.path.hint").into()),
                )
            }))
            .child(self.product_select_field(
                "settings-default-window",
                tr!(language, "select.default_window"),
                &window,
                super::select::ProductChoice::DefaultWindow,
                cx,
            ))
            .children(draft.window_custom.then(|| {
                div()
                    .flex()
                    .gap_4()
                    .child(
                        self.settings_field(
                            draft,
                            SettingsField::Width,
                            tr!(language, "settings.game.window.width.label"),
                            Some(tr!(language, "settings.game.window.width.hint").into()),
                        )
                        .flex_1(),
                    )
                    .child(
                        self.settings_field(
                            draft,
                            SettingsField::Height,
                            tr!(language, "settings.game.window.height.label"),
                            Some(tr!(language, "settings.game.window.height.hint").into()),
                        )
                        .flex_1(),
                    )
            }))
            .child(self.settings_control(
                "settings-fullscreen",
                tr!(language, "settings.game.fullscreen.label"),
                draft.fullscreen,
                UiAction::SetFullscreen,
                cx,
            ))
            .child(self.settings_field(
                draft,
                SettingsField::Jvm,
                tr!(language, "settings.game.jvm.label"),
                Some(tr!(language, "settings.game.jvm.hint").into()),
            ))
            .child(self.settings_control(
                "settings-snapshots",
                tr!(language, "settings.game.snapshots.label"),
                draft.show_snapshots,
                UiAction::ShowSnapshots,
                cx,
            ))
    }

    fn settings_downloads(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let proxy = match draft.proxy_mode {
            1 => tr!(language, "settings.downloads.proxy.direct"),
            2 => tr!(language, "settings.downloads.proxy.manual"),
            _ => tr!(language, "settings.downloads.proxy.system"),
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        self.settings_field(
                            draft,
                            SettingsField::Concurrent,
                            tr!(language, "settings.downloads.concurrent.label"),
                            Some(tr!(language, "settings.downloads.concurrent.hint").into()),
                        )
                        .flex_1(),
                    )
                    .child(
                        self.settings_field(
                            draft,
                            SettingsField::Retries,
                            tr!(language, "settings.downloads.retries.label"),
                            Some(tr!(language, "settings.downloads.retries.hint").into()),
                        )
                        .flex_1(),
                    ),
            )
            .child(self.settings_field(
                draft,
                SettingsField::Rate,
                tr!(language, "settings.downloads.rate.label"),
                Some(tr!(language, "settings.downloads.rate.hint").into()),
            ))
            .child(self.product_select_field(
                "settings-proxy",
                tr!(language, "select.proxy"),
                &proxy,
                super::select::ProductChoice::Proxy,
                cx,
            ))
            .children((draft.proxy_mode == 2).then(|| {
                self.settings_field(
                    draft,
                    SettingsField::ProxyUrl,
                    tr!(language, "settings.downloads.proxy_url.label"),
                    Some(tr!(language, "settings.downloads.proxy_url.hint").into()),
                )
            }))
            .child(self.render_connection_test(cx))
    }

    fn settings_storage(&self) -> gpui::Div {
        let language = self.language();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(note(tr!(language, "settings.storage.note")));
        match &self.shell.storage.paths {
            Ok(paths) => {
                content = content
                    .child(self.storage_location_row(
                        "settings-storage-library",
                        tr!(language, "settings.storage.library"),
                        &paths.library,
                    ))
                    .child(self.storage_location_row(
                        "settings-storage-cache",
                        tr!(language, "settings.storage.cache"),
                        &paths.cache,
                    ));
            }
            Err(error) => {
                content = content.child(
                    note(error.message(language))
                        .id("settings-storage-error")
                        .debug_selector(|| "settings-storage-error".into())
                        .text_color(rgb(self.palette.destructive)),
                );
            }
        }
        content
    }

    /// A read-only path row on the Storage information page.
    fn storage_location_row(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        path: &std::path::Path,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(label.into()),
            )
            .child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .w_full()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .bg(rgb(self.palette.secondary))
                    .text_sm()
                    .child(path.display().to_string()),
            )
    }

    fn settings_profile_mode(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let separated = self.snapshot.profile_mode == ProfileMode::Separated;
        let card = |title, description, active| {
            components::card(self.palette)
                .border_color(rgb(if active {
                    self.palette.ring
                } else {
                    self.palette.border
                }))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(title),
                )
                .child(note(description))
                .children(active.then(|| {
                    components::badge(
                        tr!(language, "settings.profile_mode.current"),
                        self.palette,
                        components::BadgeVariant::Secondary,
                    )
                }))
        };
        let mode_note = if separated {
            tr!(language, "settings.profile_mode.note_separated")
        } else {
            tr!(language, "settings.profile_mode.note_bundled")
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        card(
                            tr!(language, "settings.profile_mode.bundled.title"),
                            tr!(language, "settings.profile_mode.bundled.description"),
                            !separated,
                        )
                        .flex_1(),
                    )
                    .child(
                        card(
                            tr!(language, "settings.profile_mode.separated.title"),
                            tr!(language, "settings.profile_mode.separated.description"),
                            separated,
                        )
                        .flex_1(),
                    ),
            )
            .child(note(mode_note))
            .children((!separated).then(|| {
                self.button(
                    "separate-profiles",
                    tr!(language, "settings.profile_mode.switch"),
                    UiAction::ConfirmSeparate(true),
                    cx,
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::GameDefaults;

    #[test]
    fn defaults_reject_process_overrides_and_invalid_window_or_java_settings() {
        for defaults in [
            GameDefaults {
                memory_mib: 128,
                ..GameDefaults::default()
            },
            GameDefaults {
                width: 0,
                ..GameDefaults::default()
            },
            GameDefaults {
                height: 100_000,
                ..GameDefaults::default()
            },
            GameDefaults {
                java: "relative/java".into(),
                ..GameDefaults::default()
            },
            GameDefaults {
                jvm_arguments: "-Xmx8G".into(),
                ..GameDefaults::default()
            },
            GameDefaults {
                jvm_arguments: "-cp untrusted.jar".into(),
                ..GameDefaults::default()
            },
            GameDefaults {
                jvm_arguments: "\"unterminated".into(),
                ..GameDefaults::default()
            },
        ] {
            assert!(defaults.validate().is_err());
        }
    }

    #[gpui::test]
    fn invalid_settings_stay_visible_in_the_correct_section_without_writing_data(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            catalog.create_instance("Existing game".into(), "26.3".into(), None, None)?;
            let preferences = super::Preferences::default();
            preferences.save(root.path())?;
            let original_preferences = std::fs::read(root.path().join(crate::global_config::FILE))?;
            let original_catalog = serde_json::to_value(catalog.snapshot()?)?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.router.navigate(crate::Tab::Settings);
                view.snapshot = catalog.snapshot().unwrap_or_default();
                view.open_settings(cx);
                if let Some(draft) = &view.settings_form {
                    draft
                        .memory
                        .update(cx, |input, cx| input.set_value("not-a-number", cx));
                }
                cx.notify();
            });
            click_save(cx);
            assert!(view.read_with(cx, |view, cx| {
                view.settings_form.as_ref().is_some_and(|draft| {
                    draft.page == 1 && draft.memory.read(cx).value() == "not-a-number"
                }) && view
                    .status
                    .text(crate::Language::English)
                    .contains("Choose a memory limit between 1 and 32 GB.")
            }));
            let status = cx.debug_bounds("settings-field-Memory");
            assert!(status.is_some_and(
                |bounds| bounds.top() >= gpui::px(0.) && bounds.bottom() <= gpui::px(600.)
            ));
            view.update(cx, |view, cx| {
                if let Some(draft) = &mut view.settings_form {
                    draft
                        .memory
                        .update(cx, |input, cx| input.set_value("2", cx));
                    draft
                        .concurrent
                        .update(cx, |input, cx| input.set_value("0", cx));
                    draft.page = 0;
                }
                cx.notify();
            });
            click_save(cx);
            assert!(view.read_with(cx, |view, cx| {
                view.settings_form.as_ref().is_some_and(|draft| {
                    draft.page == 3 && draft.concurrent.read(cx).value() == "0"
                }) && view
                    .status
                    .text(crate::Language::English)
                    .contains("Choose between 1 and 16 parallel downloads.")
            }));
            assert!(cx.debug_bounds("settings-status").is_some());
            assert_eq!(
                std::fs::read(root.path().join(crate::global_config::FILE))?,
                original_preferences
            );
            assert_eq!(serde_json::to_value(catalog.snapshot()?)?, original_catalog);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    fn click_save(cx: &mut gpui::VisualTestContext) {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        // TestWindow has no display-link callback. Render the elapsed frame so
        // the simulated click uses the settled live layer's actual hitboxes.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let bounds = cx.debug_bounds("save-preferences");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
        }
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

#[cfg(test)]
#[path = "settings_draft_tests.rs"]
mod draft_tests;
