#![expect(
    clippy::redundant_pub_crate,
    reason = "Settings are shared with the parent view and engine only."
)]
//! Validated settings drafts, with defaults applied only to newly created games.

use super::scroll_area::ScrollAreaExt as _;
use std::{cell::Cell, fmt, path::PathBuf, rc::Rc};

use anyhow::{Context as _, ensure};
use block_launcher::{
    DownloadOptions, ProxyMode,
    catalog::{
        DataPolicy, Profile, ProfileAppearance, ProfileInheritance, ProfileMode, ServerPreferences,
    },
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement as _,
    ParentElement as _, ScrollHandle, Styled as _, Window, div, point, px, rgb,
};
use serde::{Deserialize, Serialize};

use super::components::{ControlState, FieldOptions};
use super::design::Icon;
use super::{
    Appearance, Block, Preferences, Tab, TextInput, UiAction, components, input, note, patterns,
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
            (256..=65536).contains(&self.memory_mib),
            "Memory must be between 256 and 65536 MiB"
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
    Library,
    Cache,
}

impl SettingsField {
    const fn page(self) -> u8 {
        match self {
            Self::Memory | Self::Java | Self::Jvm | Self::Width | Self::Height => 1,
            Self::Concurrent | Self::Retries | Self::Rate | Self::ProxyUrl => 3,
            Self::Library | Self::Cache => 4,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SettingsError {
    pub field: Option<SettingsField>,
    pub message: String,
}

impl SettingsError {
    fn at(field: SettingsField, message: impl fmt::Display) -> Self {
        Self {
            field: Some(field),
            message: message.to_string(),
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for SettingsError {}

#[derive(Clone, Debug)]
pub(super) struct StorageRequest {
    pub id: uuid::Uuid,
    pub operation: u64,
    pub draft: u64,
    pub library: String,
    pub cache: String,
    pub preferences: Preferences,
}

#[derive(Clone, Debug)]
pub(super) struct StorageState {
    pub locations: Result<block_launcher::storage::Locations, String>,
    pub cleanup: Option<block_launcher::storage::Cleanup>,
    pub error: Option<String>,
    pub pending: Option<StorageRequest>,
    pub progress: Option<block_launcher::storage::Progress>,
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            locations: Ok(Default::default()),
            cleanup: None,
            error: None,
            pending: None,
            progress: None,
        }
    }
}

pub(super) struct StorageFinished {
    pub request: StorageRequest,
    pub result: Result<String, String>,
    pub locations: Result<block_launcher::storage::Locations, String>,
    pub cleanup: Result<Option<block_launcher::storage::Cleanup>, String>,
    pub preferences: Result<Preferences, String>,
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
    pub library: Entity<TextInput>,
    pub cache: Entity<TextInput>,
    pub proxy_mode: u8,
    pub fullscreen: bool,
    pub show_snapshots: bool,
    pub appearance: Appearance,
    pub reduce_motion: bool,
    pub page: u8,
    pub error: Option<SettingsError>,
    pub notice: Option<String>,
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
            library: super::frozen_input(&self.library, cx),
            cache: super::frozen_input(&self.cache, cx),
            proxy_mode: self.proxy_mode,
            fullscreen: self.fullscreen,
            show_snapshots: self.show_snapshots,
            appearance: self.appearance,
            reduce_motion: self.reduce_motion,
            page: self.page,
            error: self.error.clone(),
            notice: self.notice.clone(),
            heading: cx.focus_handle(),
            scroll,
            navigation_scroll,
            reveal: Rc::new(Cell::new(self.reveal.get())),
        }
    }

    pub(super) fn new(preferences: &Preferences, cx: &mut Context<'_, Block>) -> Self {
        let game = &preferences.game;
        let downloads = &preferences.downloads;
        let (proxy_mode, proxy_url) = match &downloads.proxy {
            ProxyMode::System => (0, ""),
            ProxyMode::Direct => (1, ""),
            ProxyMode::Http { url } | ProxyMode::Socks5 { url } => (2, url.as_str()),
        };
        let scroll = ScrollHandle::new();
        let draft = Self {
            memory: input(&game.memory_mib.to_string(), "2048", cx),
            java: input(&game.java, "Automatic · leave empty", cx),
            java_automatic: game.java.is_empty(),
            window_custom: !WINDOW_PRESETS.contains(&(game.width, game.height)),
            jvm: input(&game.jvm_arguments, "Leave empty to use defaults", cx),
            width: input(&game.width.to_string(), "1280", cx),
            height: input(&game.height.to_string(), "720", cx),
            concurrent: input(&downloads.max_concurrent.to_string(), "8", cx),
            retries: input(&downloads.max_retries.to_string(), "2", cx),
            rate: input(
                &format_rate(downloads.bytes_per_second),
                "0 · Unlimited",
                cx,
            ),
            proxy_url: input(proxy_url, "http://127.0.0.1:8080", cx),
            library: input("", "Managed by Block", cx),
            cache: input("", "Managed by Block", cx),
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
            SettingsField::Library,
            SettingsField::Cache,
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
            SettingsField::Library => &self.library,
            SettingsField::Cache => &self.cache,
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
            4 => vec![self.library.clone(), self.cache.clone()],
            _ => Vec::new(),
        }
    }

    pub(super) fn is_dirty(&self, saved: &Preferences, cx: &gpui::App) -> bool {
        !self.parse(saved, cx).is_ok_and(|value| value == *saved)
    }

    fn storage_paths(
        &self,
        cx: &gpui::App,
    ) -> Result<block_launcher::storage::Locations, SettingsError> {
        let path = |field| {
            let value = self.input(field).read(cx).value();
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            let path = PathBuf::from(value);
            if !path.is_absolute() || value.contains('\0') {
                return Err(SettingsError::at(
                    field,
                    "Choose an absolute folder path, or leave it empty",
                ));
            }
            Ok(Some(path))
        };
        Ok(block_launcher::storage::Locations {
            library: path(SettingsField::Library)?,
            cache: path(SettingsField::Cache)?,
            ..Default::default()
        })
    }

    fn set_storage(&self, value: &block_launcher::storage::Locations, cx: &mut Context<'_, Block>) {
        for (input, path) in [(&self.library, &value.library), (&self.cache, &value.cache)] {
            let value = path
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string());
            input.update(cx, |input, cx| input.set_value(&value, cx));
        }
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
                    "Choose an absolute Java executable path, or use Automatic Java",
                ));
            }
            path
        };
        let game = GameDefaults {
            memory_mib: number(
                &self.memory,
                SettingsField::Memory,
                "Memory must be a whole number of MiB",
                cx,
            )?,
            java,
            jvm_arguments: self.jvm.read(cx).value(),
            width: number(
                &self.width,
                SettingsField::Width,
                "Window width must be a whole number",
                cx,
            )?,
            height: number(
                &self.height,
                SettingsField::Height,
                "Window height must be a whole number",
                cx,
            )?,
            fullscreen: self.fullscreen,
            show_snapshots: self.show_snapshots,
        };
        for (valid, field, message) in [
            (
                (256..=65536).contains(&game.memory_mib),
                SettingsField::Memory,
                "Memory must be between 256 and 65536 MiB",
            ),
            (
                (320..=16384).contains(&game.width),
                SettingsField::Width,
                "Window width must be between 320 and 16384 pixels",
            ),
            (
                (240..=16384).contains(&game.height),
                SettingsField::Height,
                "Window height must be between 240 and 16384 pixels",
            ),
        ] {
            if !valid {
                return Err(SettingsError::at(field, message));
            }
        }
        GameDefaults {
            java: game.java.clone(),
            ..GameDefaults::default()
        }
        .validate()
        .map_err(|error| SettingsError::at(SettingsField::Java, error))?;
        game.validate()
            .map_err(|error| SettingsError::at(SettingsField::Jvm, error))?;
        Ok(game)
    }

    pub(super) fn parse_downloads(&self, cx: &gpui::App) -> Result<DownloadOptions, SettingsError> {
        let concurrent = number(
            &self.concurrent,
            SettingsField::Concurrent,
            "Concurrent downloads must be a whole number",
            cx,
        )?;
        let retries = number(
            &self.retries,
            SettingsField::Retries,
            "Retries must be a whole number",
            cx,
        )?;
        if !(1..=16).contains(&concurrent) {
            return Err(SettingsError::at(
                SettingsField::Concurrent,
                "download concurrency must be 1..=16",
            ));
        }
        if retries > 10 {
            return Err(SettingsError::at(
                SettingsField::Retries,
                "download retries must be 0..=10",
            ));
        }
        let rate = parse_rate(&self.rate.read(cx).value())
            .map_err(|error| SettingsError::at(SettingsField::Rate, error))?;
        let proxy = match self.proxy_mode {
            0 => ProxyMode::System,
            1 => ProxyMode::Direct,
            2 => ProxyMode::manual(self.proxy_url.read(cx).value().trim().into())
                .map_err(|error| SettingsError::at(SettingsField::ProxyUrl, error))?,
            _ => {
                return Err(SettingsError::at(
                    SettingsField::ProxyUrl,
                    "Choose System, Direct, or Manual proxy",
                ));
            }
        };
        let downloads = DownloadOptions {
            max_concurrent: concurrent,
            max_retries: retries,
            bytes_per_second: rate,
            proxy,
        };
        downloads
            .validate()
            .map_err(|error| SettingsError::at(SettingsField::ProxyUrl, error))?;
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
                    (&self.memory, defaults.game.memory_mib.to_string()),
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
            4 => self.set_storage(&block_launcher::storage::Locations::default(), cx),
            _ => return,
        }
        self.error = None;
        self.notice = Some("Defaults restored in this section. Save changes to keep them.".into());
    }
}

fn number<T: std::str::FromStr>(
    input: &Entity<TextInput>,
    field: SettingsField,
    message: &'static str,
    cx: &gpui::App,
) -> Result<T, SettingsError> {
    input
        .read(cx)
        .value()
        .trim()
        .parse()
        .map_err(|_| SettingsError::at(field, message))
}

/// Decimal MB/s, with at most six fractional digits, maps exactly to whole bytes.
fn parse_rate(value: &str) -> anyhow::Result<Option<u64>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.len() <= 6,
        "Use a non-negative MB/s value with at most six decimal places"
    );
    let whole: u64 = whole.parse().context("Download speed is too large")?;
    let fractional: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse()?
    };
    let scale = 10u64.pow(u32::try_from(6 - fraction.len())?);
    let bytes = whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fractional * scale))
        .context("Download speed is too large")?;
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
        if self.shell.storage.pending.is_none()
            && let Some(engine) = &self.engine
        {
            self.shell.storage.locations = block_launcher::storage::Locations::load(&engine.root)
                .and_then(|value| value.validate(&engine.root))
                .map_err(|error| format!("{error:#}"));
            match block_launcher::storage::cleanup_status(&engine.root) {
                Ok(cleanup) => self.shell.storage.cleanup = cleanup,
                Err(error) => self.shell.storage.error = Some(format!("{error:#}")),
            }
        }
        let mut draft = SettingsDraft::new(&self.saved_preferences, cx);
        if let Ok(locations) = &self.shell.storage.locations {
            draft.set_storage(locations, cx);
        }
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

    pub(super) fn reset_settings(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(draft) = &mut self.settings_form {
            draft.restore_page(cx);
            self.preferences.appearance = draft.appearance;
            self.preferences.reduce_motion = draft.reduce_motion;
        }
    }

    pub(super) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.storage.pending.is_some() {
            self.status = "Storage is busy. Wait for relocation or cancel it before saving.".into();
            return;
        }
        let storage_dirty = self.storage_dirty(cx);
        if storage_dirty {
            self.apply_storage(false, window, cx);
            return;
        }
        let parsed = self
            .settings_form
            .as_ref()
            .map(|draft| draft.parse(&self.saved_preferences, cx));
        let Some(parsed) = parsed else {
            return;
        };
        match parsed {
            Ok(preferences) => {
                let result = self
                    .engine
                    .as_ref()
                    .context("Settings could not be saved: engine unavailable")
                    .and_then(|engine| preferences.save(&engine.root));
                match result {
                    Ok(()) => {
                        let page = self.settings_form.as_ref().map_or(0, |draft| draft.page);
                        self.preferences.clone_from(&preferences);
                        self.saved_preferences = preferences;
                        self.open_settings(cx);
                        if let Some(draft) = &mut self.settings_form {
                            draft.page = page;
                            draft.notice = Some(
                                "Settings saved. New game defaults apply to new setups.".into(),
                            );
                        }
                        self.status = "Settings saved · game defaults apply to new games".into();
                        if let Some(engine) = &self.engine {
                            engine.load_versions();
                        }
                    }
                    Err(error) => self.settings_failure(
                        SettingsError {
                            field: None,
                            message: format!("Could not save settings: {error:#}"),
                        },
                        window,
                        cx,
                    ),
                }
            }
            Err(error) => self.settings_failure(error, window, cx),
        }
    }

    fn storage_dirty(&self, cx: &gpui::App) -> bool {
        self.settings_form.as_ref().is_some_and(|draft| {
            match (&self.shell.storage.locations, draft.storage_paths(cx)) {
                (Ok(saved), Ok(value)) => {
                    saved.library != value.library || saved.cache != value.cache
                }
                (Err(_), _) => {
                    draft.page == 4
                        || !draft.library.read(cx).value().is_empty()
                        || !draft.cache.read(cx).value().is_empty()
                }
                _ => true,
            }
        })
    }

    pub(super) fn move_storage_games(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.storage_dirty(cx) {
            self.report("Save the destination folders before moving existing games.".into());
            return;
        }
        self.apply_storage(true, window, cx);
    }

    fn apply_storage(&mut self, move_games: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy || self.shell.storage.pending.is_some() {
            return;
        }
        let Some(draft) = &self.settings_form else {
            return;
        };
        if draft
            .fields()
            .iter()
            .any(|field| field.read(cx).is_composing())
        {
            return;
        }
        if let Err(error) = &self.shell.storage.locations {
            self.settings_failure(
                SettingsError {
                    field: Some(SettingsField::Library),
                    message: error.clone(),
                },
                window,
                cx,
            );
            return;
        }
        let parsed = draft.storage_paths(cx).and_then(|locations| {
            draft
                .parse(&self.saved_preferences, cx)
                .map(|preferences| (locations, preferences))
        });
        let (locations, preferences) = match parsed {
            Ok(value) => value,
            Err(error) => {
                self.settings_failure(error, window, cx);
                return;
            }
        };
        let validation = self
            .engine
            .as_ref()
            .context("Storage engine is unavailable")
            .and_then(|engine| locations.validate(&engine.root));
        if let Err(error) = validation {
            self.settings_failure(
                SettingsError {
                    field: Some(SettingsField::Library),
                    message: error.to_string(),
                },
                window,
                cx,
            );
            return;
        }
        if self.shell.storage.cleanup.is_some() {
            self.report("Keep or remove the retained copy before another storage change.".into());
            return;
        }
        let request = StorageRequest {
            id: uuid::Uuid::new_v4(),
            operation: 0,
            draft: draft.library.entity_id().as_u64(),
            library: draft.library.read(cx).value(),
            cache: draft.cache.read(cx).value(),
            preferences: preferences.clone(),
        };
        let saved = self
            .engine
            .as_ref()
            .context("Storage engine is unavailable")
            .and_then(|engine| preferences.save(&engine.root));
        if let Err(error) = saved {
            self.settings_failure(
                SettingsError {
                    field: None,
                    message: error.to_string(),
                },
                window,
                cx,
            );
            return;
        }
        self.preferences.clone_from(&preferences);
        self.saved_preferences = preferences;
        self.begin_cancellable("Preparing storage relocation");
        let mut request = request;
        request.operation = self.operation_revision;
        self.shell.storage.pending = Some(request.clone());
        self.shell.storage.error = None;
        self.shell.storage.progress = Some(block_launcher::storage::Progress {
            phase: "Preparing",
            completed_bytes: 0,
            total_bytes: 0,
        });
        if let Some(engine) = &self.engine {
            engine.relocate_storage(request, locations, move_games, self.cancel.clone());
        }
        cx.notify();
    }

    pub(super) fn clean_storage(
        &mut self,
        id: uuid::Uuid,
        remove: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy
            || self
                .shell
                .storage
                .cleanup
                .as_ref()
                .is_none_or(|state| state.id != id)
        {
            return;
        }
        let request = StorageRequest {
            id: uuid::Uuid::new_v4(),
            operation: 0,
            draft: 0,
            library: String::new(),
            cache: String::new(),
            preferences: self.saved_preferences.clone(),
        };
        if matches!(
            self.shell.surfaces.top().map(|entry| &entry.kind),
            Some(super::shell::Surface::StorageCleanup(_))
        ) {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
        self.begin(if remove {
            "Removing the confirmed retained copy"
        } else {
            "Keeping the retained copy"
        });
        let mut request = request;
        request.operation = self.operation_revision;
        self.shell.storage.pending = Some(request.clone());
        self.shell.storage.error = None;
        self.shell.storage.progress = Some(block_launcher::storage::Progress {
            phase: if remove {
                "Removing retained copy"
            } else {
                "Keeping retained copy"
            },
            completed_bytes: 0,
            total_bytes: 0,
        });
        if let Some(engine) = &self.engine {
            engine.cleanup_storage(request, id, remove);
        }
        cx.notify();
    }

    pub(super) fn storage_progress(
        &mut self,
        id: uuid::Uuid,
        progress: block_launcher::storage::Progress,
    ) {
        if self
            .shell
            .storage
            .pending
            .as_ref()
            .is_some_and(|request| request.id == id)
        {
            self.status = format!("{} storage", progress.phase);
            self.shell.storage.progress = Some(progress);
        }
    }

    pub(super) fn storage_finished(
        &mut self,
        finished: StorageFinished,
        cx: &mut Context<'_, Self>,
    ) {
        let request = &finished.request;
        if self
            .shell
            .storage
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != request.id)
        {
            return;
        }
        let result = finished.result.and_then(|message| {
            for state in [
                finished.locations.as_ref().map(|_| ()),
                finished.cleanup.as_ref().map(|_| ()),
                finished.preferences.as_ref().map(|_| ()),
            ] {
                if let Err(error) = state {
                    return Err(format!(
                        "Storage operation ended, but its saved state could not be read: {error}"
                    ));
                }
            }
            Ok(message)
        });
        let unchanged = self.settings_form.as_ref().is_some_and(|draft| {
            draft.library.entity_id().as_u64() == request.draft
                && draft.library.read(cx).value() == request.library
                && draft.cache.read(cx).value() == request.cache
                && draft
                    .parse(&request.preferences, cx)
                    .is_ok_and(|value| value == request.preferences)
        });
        self.shell.storage.pending = None;
        self.shell.storage.progress = None;
        self.shell.storage.locations = finished.locations;
        match finished.cleanup {
            Ok(cleanup) => self.shell.storage.cleanup = cleanup,
            Err(error) => self.shell.storage.error = Some(error),
        }
        if let Ok(preferences) = finished.preferences {
            self.saved_preferences = preferences;
        }
        if self.operation_revision == request.operation {
            self.busy = self.active_run.is_some();
            self.operation_cancellable = false;
        }
        match result {
            Ok(message) => {
                if unchanged {
                    let page = self.settings_form.as_ref().map_or(4, |draft| draft.page);
                    self.preferences.clone_from(&self.saved_preferences);
                    self.open_settings(cx);
                    if let Some(draft) = &mut self.settings_form {
                        draft.page = page;
                        draft.notice = Some(message.clone());
                    }
                }
                self.status = message.clone();
                self.push_toast(message, super::feedback::ToastKind::Success);
            }
            Err(error) => {
                self.shell.storage.error = Some(error.clone());
                self.report(format!("Storage change failed: {error}. Saved preferences were kept; review the retained-copy status."));
            }
        }
        cx.notify();
    }

    pub(super) fn settings_failure(
        &mut self,
        error: SettingsError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.report(format!("Could not save settings: {}", error.message));
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
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl gpui::IntoElement + use<> {
        let viewport = window.viewport_size();
        let narrow = patterns::is_narrow(viewport);
        let Some(draft) = &self.settings_form else {
            return div().into_any_element();
        };
        let navigation = self.settings_navigation(draft, viewport, cx);
        let detail = self.transition_content(
            super::motion::Scope::Detail,
            self.render_settings_detail(window, cx),
            window,
            cx,
        );
        if narrow {
            div()
                .id("settings-page")
                .size_full()
                .min_h_0()
                .flex()
                .child(
                    div()
                        .w(px(56.))
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(rgb(self.palette.border))
                        .child(navigation),
                )
                .child(div().flex_1().min_w_0().min_h_0().child(detail))
                .into_any_element()
        } else {
            patterns::management_layout(
                viewport,
                navigation.into_any_element(),
                detail.into_any_element(),
                true,
                self.palette,
            )
            .id("settings-page")
            .into_any_element()
        }
    }

    pub(super) fn render_settings_detail(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        self.settings_form.as_ref().map_or_else(
            || div().into_any_element(),
            |draft| {
                self.settings_detail(draft, window.viewport_size(), cx)
                    .into_any_element()
            },
        )
    }

    fn settings_navigation(
        &self,
        draft: &SettingsDraft,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let narrow = patterns::is_narrow(viewport);
        let page = draft.page;
        let dirty = draft.is_dirty(&self.saved_preferences, cx) || self.storage_dirty(cx);
        let categories = [
            (0, "general", "General", Icon::System),
            (1, "game", "Game defaults", Icon::Settings),
            (2, "profiles", "Profile mode", Icon::Games),
            (3, "downloads", "Downloads & network", Icon::Download),
            (4, "storage", "Storage", Icon::Folder),
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
                        "Settings categories",
                        self.palette,
                        false,
                    )
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(categories.map(|(index, id, label, icon)| {
                        let mut button = components::button(
                            format!("settings-page-{id}"),
                            if narrow { "" } else { label },
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
                        .flex_row_reverse()
                        .justify_end()
                        .child(icon.render(16., self.palette.muted));
                        if narrow {
                            button = button.justify_center().px_0().gap_0();
                        }
                        if page == index {
                            button = button
                                .bg(rgb(self.palette.accent))
                                .text_color(rgb(self.palette.foreground));
                        }
                        button
                    })),
            )
            .child(
                patterns::management_footer(viewport, self.palette)
                    .justify_center()
                    .child(if narrow {
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgb(if dirty {
                                self.palette.warning
                            } else {
                                self.palette.muted
                            }))
                            .into_any_element()
                    } else {
                        note(if dirty {
                            "Unsaved changes"
                        } else {
                            "All changes saved"
                        })
                        .into_any_element()
                    }),
            )
    }

    fn settings_detail(
        &self,
        draft: &SettingsDraft,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let narrow = patterns::is_narrow(viewport);
        let page = draft.page;
        let (title, description) = match page {
            1 => (
                "Game defaults",
                "A comfortable starting point for new game setups.",
            ),
            2 => (
                "Profile mode",
                "Choose how Instances and Profiles work together.",
            ),
            3 => (
                "Downloads & network",
                "Control download limits and connection preferences.",
            ),
            4 => ("Storage", "Files managed by this Block installation."),
            _ => ("General", "Make Block feel at home."),
        };
        let feedback = draft
            .error
            .as_ref()
            .map(|error| (error.message.clone(), true))
            .or_else(|| draft.notice.clone().map(|message| (message, false)));
        let body = self.settings_section_body(draft, narrow, cx);
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
                    .scroll_area(&draft.scroll, "Settings fields", self.palette, false)
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(608.))
                            .p(px(if narrow { 16. } else { 32. }))
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .track_focus(&draft.heading)
                                            .child(title),
                                    )
                                    .child(note(description)),
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
            .child(self.settings_footer(draft, viewport, cx))
    }

    fn settings_section_body(
        &self,
        draft: &SettingsDraft,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        match draft.page {
            1 => self.settings_fields(draft, narrow, cx),
            2 => self.settings_profile_mode(narrow, cx),
            3 => self.settings_downloads(draft, narrow, cx),
            4 => self.settings_storage(draft, cx),
            _ => self.settings_appearance(draft, cx),
        }
        .into_any_element()
    }

    fn settings_footer(
        &self,
        draft: &SettingsDraft,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let narrow = patterns::is_narrow(viewport);
        let page = draft.page;
        let dirty = draft.is_dirty(&self.saved_preferences, cx) || self.storage_dirty(cx);
        let mut footer = patterns::management_footer(viewport, self.palette).justify_between();
        if page == 2 {
            footer = footer.child(
                note(if narrow {
                    "Immediate"
                } else {
                    "Mode changes apply immediately."
                })
                .flex_1(),
            );
        } else {
            footer = footer.child(self.button(
                "reset-preferences",
                if narrow {
                    "Restore"
                } else {
                    "Restore defaults"
                },
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
                    "Cancel",
                    if self.tab == Tab::Onboarding {
                        UiAction::Drawer(None)
                    } else {
                        UiAction::Tab(Tab::Home)
                    },
                    cx,
                ))
                .child(components::button(
                    "save-preferences",
                    "Save changes",
                    self.palette,
                    components::ButtonVariant::Primary,
                    ControlState {
                        disabled: !dirty
                            || self.shell.storage.pending.is_some()
                            || (self.busy && self.storage_dirty(cx)),
                        busy: self.shell.storage.pending.is_some(),
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
        label: &'static str,
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
        label: &'static str,
        hint: Option<&'static str>,
    ) -> gpui::Stateful<gpui::Div> {
        let error = draft
            .error
            .as_ref()
            .filter(|error| error.field == Some(field))
            .map(|error| error.message.clone().into());
        patterns::reveal_field(
            components::field_with(
                label,
                draft.input(field),
                self.palette,
                FieldOptions {
                    hint: hint.map(Into::into),
                    error,
                    required: false,
                },
            ),
            draft.scroll.clone(),
            draft.reveal.clone(),
            field,
        )
        .id(gpui::SharedString::from(format!(
            "settings-field-{field:?}"
        )))
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
        let Ok(value) = memory.value().trim().parse::<u32>() else {
            return;
        };
        if !(256..=65536).contains(&value) {
            return;
        }
        let next = if increase {
            value.saturating_add(256).min(65536)
        } else {
            value.saturating_sub(256).max(256)
        };
        draft
            .memory
            .update(cx, |input, cx| input.set_value(&next.to_string(), cx));
        draft.notice = None;
        cx.notify();
    }

    fn settings_memory_field(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let value = draft
            .memory
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| (256..=65536).contains(value));
        let composing = draft.memory.read(cx).is_composing();
        let friendly = value.map_or_else(
            || "Enter a whole number of MiB.".to_owned(),
            |value| {
                if value % 1024 == 0 {
                    format!("{} GiB ({value} MiB)", value / 1024)
                } else {
                    format!(
                        "Approximately {:.3} GiB ({value} MiB)",
                        f64::from(value) / 1024.
                    )
                }
            },
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
                                value == 65536
                            } else {
                                value == 256
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
        div().min_w_0().flex().flex_col().gap_2()
            .child(self.settings_field(draft, SettingsField::Memory, "Default memory (MiB)", Some("Allowed: 256–65536 MiB. Step: 256 MiB. Arrow Up/Down adjusts the focused value.")))
            .child(div().flex().items_center().gap_2().child(step(false)).child(step(true))
                .child(note(friendly).id("settings-memory-friendly")))
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers == gpui::Modifiers::none()
                    && matches!(event.keystroke.key.as_str(), "up" | "down")
                    && view.settings_form.as_ref().is_some_and(|draft| !draft.memory.read(cx).is_composing() && draft.memory.read(cx).focus_handle(cx).is_focused(window)) {
                    view.adjust_settings_memory(event.keystroke.key == "up", cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn settings_appearance(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let appearance = match draft.appearance {
            Appearance::System => "Follow system",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        };
        div().flex().flex_col().gap_6()
            .child(self.product_select_field("settings-appearance", "Appearance", appearance, super::select::ProductChoice::Appearance, cx))
            .child(div().flex().flex_col().gap_2()
                .child(self.settings_control("reduce-motion", "Reduce animations", draft.reduce_motion, UiAction::ReduceMotion, cx))
                .child(note("Use simpler transitions throughout Block. Your system’s reduced-motion preference is always respected.")))
            .child(note("Appearance previews immediately. Save changes keeps this choice; Cancel restores your saved appearance."))
    }

    fn settings_fields(
        &self,
        draft: &SettingsDraft,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut runtime = div().flex().gap_4();
        let mut dimensions = div().flex().gap_4();
        if narrow {
            runtime = runtime.flex_col();
            dimensions = dimensions.flex_col();
        }
        let java = if draft.java_automatic {
            "Automatic Java"
        } else {
            "Custom executable"
        };
        let window = format!(
            "{} × {}{}",
            draft.width.read(cx).value(),
            draft.height.read(cx).value(),
            if draft.window_custom {
                " · Custom"
            } else {
                ""
            }
        );
        div().flex().flex_col().gap_5()
            .child(note("Defaults apply to new Instances and Profiles. Existing setups keep their current settings.").p_3().rounded_lg().bg(rgb(self.palette.secondary)))
            .child(runtime.child(self.settings_memory_field(draft, cx).flex_1())
                .child(self.product_select_field("settings-default-java", "Default Java runtime", java, super::select::ProductChoice::DefaultJava, cx).flex_1()))
            .children((!draft.java_automatic).then(|| self.settings_field(draft, SettingsField::Java, "Java executable path", Some("Absolute path to the Java executable used for new games."))))
            .child(self.product_select_field("settings-default-window", "Default window size", &window, super::select::ProductChoice::DefaultWindow, cx))
            .children(draft.window_custom.then(|| dimensions.child(self.settings_field(draft, SettingsField::Width, "Window width (px)", Some("Allowed: 320–16384 pixels.")).flex_1())
                .child(self.settings_field(draft, SettingsField::Height, "Window height (px)", Some("Allowed: 240–16384 pixels.")).flex_1())))
            .child(self.settings_control("settings-fullscreen", "Start games in fullscreen", draft.fullscreen, UiAction::SetFullscreen, cx))
            .child(self.settings_field(draft, SettingsField::Jvm, "Default extra JVM arguments", Some("Memory is set above. Leave empty unless you need additional launch arguments.")))
            .child(self.settings_control("settings-snapshots", "Include snapshot versions", draft.show_snapshots, UiAction::ShowSnapshots, cx))
    }

    fn settings_downloads(
        &self,
        draft: &SettingsDraft,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut limits = div().flex().gap_4();
        if narrow {
            limits = limits.flex_col();
        }
        div().flex().flex_col().gap_5()
            .child(limits.child(self.settings_field(draft, SettingsField::Concurrent, "Parallel downloads", Some("1–16 active object transfers.")).flex_1())
                .child(self.settings_field(draft, SettingsField::Retries, "Retry count", Some("0–10 additional attempts.")).flex_1()))
            .child(self.settings_field(draft, SettingsField::Rate, "Download speed limit (MB/s)", Some("0 or empty means unlimited. 1 MB/s = 1,000,000 bytes/s; metadata is not throttled.")))
            .child(self.product_select_field("settings-proxy", "Proxy", match draft.proxy_mode {
                1 => "Direct connection", 2 => "Manual proxy", _ => "Use system proxy",
            }, super::select::ProductChoice::Proxy, cx))
            .children((draft.proxy_mode == 2).then(|| self.settings_field(draft, SettingsField::ProxyUrl, "Proxy address", Some("HTTP, HTTPS and SOCKS5 URLs are supported. Use socks5h:// to resolve hostnames through the proxy. Credentials are unsupported."))))
            .child(self.render_connection_test(cx))
    }

    fn settings_storage(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let state = &self.shell.storage;
        let mut content = div().flex().flex_col().gap_5();
        if let Some(progress) = &state.progress {
            content = content.child(
                components::card(self.palette)
                    .child(div().text_sm().child(format!("{} storage", progress.phase)))
                    .child(components::progress(
                        if progress.total_bytes == 0 {
                            components::ProgressState::Indeterminate
                        } else {
                            components::ProgressState::Measured {
                                completed: progress.completed_bytes,
                                total: progress.total_bytes,
                            }
                        },
                        self.palette,
                    ))
                    .children((progress.total_bytes > 0).then(|| {
                        note(format!(
                            "{} / {} bytes",
                            progress.completed_bytes, progress.total_bytes
                        ))
                    }))
                    .children(self.operation_cancellable.then(|| {
                        self.button("storage-cancel", "Cancel relocation", UiAction::Stop, cx)
                    })),
            );
        }
        if let Some(error) = state.error.as_ref().or(state.locations.as_ref().err()) {
            content = content.child(note(error.clone()).text_color(rgb(self.palette.destructive)));
        }
        content = content
            .child(self.settings_field(draft, SettingsField::Library, "Default library folder", Some("New Instances get their own subfolder here. Existing game folders are kept in place.")))
            .child(self.settings_field(draft, SettingsField::Cache, "Download cache folder", Some("Reusable downloads are copied and verified before this location changes.")))
            .child(note("Leave paths empty to use folders managed by Block. Previous copies stay on disk until you review them."));
        if self.busy && state.pending.is_none() {
            content = content.child(note(
                "Finish downloads and stop games before changing storage.",
            ));
        }
        if let Some(cleanup) = &state.cleanup {
            content = content.child(components::card(self.palette)
                .child(div().text_sm().child(if cleanup.interrupted { "Incomplete destination copy retained" } else { "Previous copy retained" }))
                .child(note(if cleanup.interrupted { "The move was rolled back. Original files are still in use. Review the incomplete copy before removing it." }
                    else { "The new locations are in use. Review the previous copy before removing it, or keep it." }))
                .child(self.button("storage-review-cleanup", "Review retained copy…", UiAction::StorageReviewCleanup, cx))
                .child(self.button("storage-keep-copy", "Keep copy", UiAction::StorageCleanup(cleanup.id, false), cx)));
        } else if state.pending.is_none() {
            let movable = self
                .engine
                .as_ref()
                .zip(state.locations.as_ref().ok())
                .is_some_and(|(engine, locations)| {
                    self.snapshot.instances.iter().any(|instance| {
                        instance
                            .directory
                            .file_name()
                            .is_some_and(|name| name == instance.id.as_str())
                            && instance.directory
                                != locations.library(&engine.root).join(&instance.id)
                    })
                });
            if movable {
                content = content.child(note("Saving the default library does not move existing games. This separate action moves managed games to the saved library; custom folders stay in place."))
                    .child(self.button("storage-move-games", "Move existing managed games", UiAction::StorageMoveGames, cx));
            }
        }
        content
    }

    pub(super) fn render_storage_confirmation(
        &self,
        id: uuid::Uuid,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let Some(cleanup) = self
            .shell
            .storage
            .cleanup
            .as_ref()
            .filter(|state| state.id == id)
        else {
            return div().child(note("The retained copy changed. Close this window and review the current storage state."));
        };
        let mut content = div().flex().flex_col().gap_3()
            .child(div().text_xl().child("Remove this retained copy?"))
            .child(note(if cleanup.interrupted { "Only the incomplete destination folders listed below will be removed. The original locations remain in use." }
                else { "Only the previous managed folders listed below will be removed. The verified new copy remains in use." }));
        for path in &cleanup.previous_folders {
            content = content.child(note(path.display().to_string()));
        }
        if cleanup.cleanup_started {
            content = content.child(note("An earlier cleanup stopped partway through. Confirm again to remove its remaining files."));
        }
        content.child(note("Keep the copy if you want to inspect it first. Edited previous copies are preserved and require manual review."))
            .child(self.button("storage-confirm-remove", "Remove retained copy", UiAction::StorageCleanup(id, true), cx))
            .child(self.button("storage-confirm-keep", "Keep copy", UiAction::StorageCleanup(id, false), cx))
    }

    fn settings_profile_mode(&self, narrow: bool, cx: &Context<'_, Self>) -> gpui::Div {
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
                    components::badge("Current", self.palette, components::BadgeVariant::Secondary)
                }))
        };
        let mut modes = div().flex().gap_3();
        if narrow {
            modes = modes.flex_col();
        }
        div().flex().flex_col().gap_4()
            .child(modes.child(card("Bundled Profile", "Recommended for beginners. One Instance has one Profile and its own launch preferences.", !separated).flex_1())
                .child(card("Separated Profile", "Reuse an Instance across several Profiles with independent accounts, memory and JVM arguments.", separated).flex_1()))
            .child(note(if separated { "Your existing setups are preserved. Returning to Bundled is unavailable, including through Restore defaults." }
                else { "You can switch to Separated Profile once. Existing Instances, Profiles and files stay in place. This cannot be reversed." }))
            .children((!separated).then(|| self.button("separate-profiles", "Switch to Separated Profile…", UiAction::ConfirmSeparate(true), cx)))
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
            let original_preferences = std::fs::read(root.path().join("preferences.json"))?;
            let original_catalog = std::fs::read(root.path().join("catalog.json"))?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.tab = crate::Tab::Settings;
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
                }) && view.status.contains("Memory must be a whole number")
            }));
            let status = cx.debug_bounds("settings-field-Memory");
            assert!(status.is_some_and(
                |bounds| bounds.top() >= gpui::px(0.) && bounds.bottom() <= gpui::px(600.)
            ));
            view.update(cx, |view, cx| {
                if let Some(draft) = &mut view.settings_form {
                    draft
                        .memory
                        .update(cx, |input, cx| input.set_value("2048", cx));
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
                }) && view.status.contains("concurrency must be 1..=16")
            }));
            assert!(cx.debug_bounds("settings-status").is_some());
            assert_eq!(
                std::fs::read(root.path().join("preferences.json"))?,
                original_preferences
            );
            assert_eq!(
                std::fs::read(root.path().join("catalog.json"))?,
                original_catalog
            );
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
