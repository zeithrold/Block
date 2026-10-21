#![expect(
    clippy::redundant_pub_crate,
    reason = "Settings are shared with the parent view and engine only."
)]
//! Validated settings drafts, with defaults applied only to newly created games.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context as _, ensure};
use block_launcher::{
    DownloadOptions, ProxyMode,
    catalog::{DataPolicy, Profile, ProfileMode},
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::{
    Animation, AnimationExt as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px, rgb,
};
use serde::{Deserialize, Serialize};

use super::{Appearance, Block, Preferences, Tab, TextInput, UiAction, field, input, note};

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

#[derive(Debug)]
pub(super) struct SettingsDraft {
    pub memory: Entity<TextInput>,
    pub java: Entity<TextInput>,
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
    pub page: u8,
}

impl SettingsDraft {
    pub(super) fn new(preferences: &Preferences, cx: &mut Context<'_, Block>) -> Self {
        let game = &preferences.game;
        let downloads = &preferences.downloads;
        let (proxy_mode, proxy_url) = match &downloads.proxy {
            ProxyMode::System => (0, ""),
            ProxyMode::Direct => (1, ""),
            ProxyMode::Http { url } => (2, url.as_str()),
        };
        Self {
            memory: input(&game.memory_mib.to_string(), "2048", cx),
            java: input(&game.java, "Automatic · leave empty", cx),
            jvm: input(&game.jvm_arguments, "Optional quoted JVM arguments", cx),
            width: input(&game.width.to_string(), "1280", cx),
            height: input(&game.height.to_string(), "720", cx),
            concurrent: input(&downloads.max_concurrent.to_string(), "8", cx),
            retries: input(&downloads.max_retries.to_string(), "2", cx),
            rate: input(
                &downloads
                    .bytes_per_second
                    .map_or_else(String::new, |value| value.to_string()),
                "Unlimited · leave empty",
                cx,
            ),
            proxy_url: input(proxy_url, "http://127.0.0.1:8080", cx),
            proxy_mode,
            fullscreen: game.fullscreen,
            show_snapshots: game.show_snapshots,
            page: 0,
        }
    }

    pub(super) fn fields(&self) -> Vec<Entity<TextInput>> {
        match self.page {
            1 => vec![
                self.memory.clone(),
                self.java.clone(),
                self.jvm.clone(),
                self.width.clone(),
                self.height.clone(),
            ],
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

    pub(super) fn parse(
        &self,
        current: &Preferences,
        cx: &Context<'_, Block>,
    ) -> anyhow::Result<Preferences> {
        let mut preferences = current.clone();
        preferences.game = self.parse_game(cx).context("Game defaults")?;
        preferences.downloads = self.parse_downloads(cx).context("Downloads & network")?;
        Ok(preferences)
    }

    fn parse_game(&self, cx: &Context<'_, Block>) -> anyhow::Result<GameDefaults> {
        let game = GameDefaults {
            memory_mib: number(&self.memory, "Memory must be a whole number of MiB", cx)?,
            java: self.java.read(cx).value().trim().into(),
            jvm_arguments: self.jvm.read(cx).value(),
            width: number(&self.width, "Window width must be a whole number", cx)?,
            height: number(&self.height, "Window height must be a whole number", cx)?,
            fullscreen: self.fullscreen,
            show_snapshots: self.show_snapshots,
        };
        game.validate()?;
        Ok(game)
    }

    fn parse_downloads(&self, cx: &Context<'_, Block>) -> anyhow::Result<DownloadOptions> {
        let rate = self.rate.read(cx).value();
        let downloads = DownloadOptions {
            max_concurrent: number(
                &self.concurrent,
                "Concurrent downloads must be a whole number",
                cx,
            )?,
            max_retries: number(&self.retries, "Retries must be a whole number", cx)?,
            bytes_per_second: if rate.trim().is_empty() {
                None
            } else {
                Some(
                    rate.trim()
                        .parse()
                        .context("Rate must be positive bytes per second or empty for unlimited")?,
                )
            },
            proxy: match self.proxy_mode {
                0 => ProxyMode::System,
                1 => ProxyMode::Direct,
                2 => ProxyMode::Http {
                    url: self.proxy_url.read(cx).value().trim().into(),
                },
                _ => anyhow::bail!("Choose System, Direct, or HTTP proxy"),
            },
        };
        downloads.validate()?;
        Ok(downloads)
    }
}

fn number<T: std::str::FromStr>(
    input: &Entity<TextInput>,
    message: &'static str,
    cx: &Context<'_, Block>,
) -> anyhow::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    input.read(cx).value().trim().parse().context(message)
}

impl Block {
    pub(super) fn open_settings(&mut self, cx: &mut Context<'_, Self>) {
        self.settings_form = Some(SettingsDraft::new(&self.preferences, cx));
    }

    pub(super) fn reset_settings(&mut self, cx: &mut Context<'_, Self>) {
        let page = self.settings_form.as_ref().map_or(0, |draft| draft.page);
        self.preferences = Preferences {
            onboarding_complete: self.saved_preferences.onboarding_complete,
            ..Preferences::default()
        };
        self.open_settings(cx);
        if let Some(draft) = &mut self.settings_form {
            draft.page = page;
        }
        self.status = "Defaults restored in draft · Save changes to keep them".into();
    }

    pub(super) fn save_settings(&mut self, cx: &Context<'_, Self>) {
        let result = (|| {
            let draft = self
                .settings_form
                .as_ref()
                .context("Open Settings before saving")?;
            let preferences = draft.parse(&self.preferences, cx)?;
            let engine = self
                .engine
                .as_ref()
                .context("Settings could not be saved: engine unavailable")?;
            preferences.save(&engine.root)?;
            Ok::<_, anyhow::Error>(preferences)
        })();
        match result {
            Ok(preferences) => {
                self.preferences = preferences.clone();
                self.saved_preferences = preferences;
                self.status = "Settings saved · game defaults apply to new games".into();
                if let Some(engine) = &self.engine {
                    engine.load_versions();
                }
            }
            Err(error) => {
                let message = format!("{error:#}");
                if let Some(draft) = &mut self.settings_form {
                    if message.starts_with("Game defaults:") {
                        draft.page = 1;
                    } else if message.starts_with("Downloads & network:") {
                        draft.page = 3;
                    }
                }
                self.report(format!("Could not save settings: {message}"));
            }
        }
    }

    pub(super) fn render_settings(&self, cx: &Context<'_, Self>) -> impl gpui::IntoElement + use<> {
        let page = self.settings_form.as_ref().map_or(0, |draft| draft.page);
        let body = match page {
            1 => self
                .settings_form
                .as_ref()
                .map_or_else(div, |draft| self.settings_fields(draft, cx)),
            2 => self.settings_profile_mode(cx),
            3 => self
                .settings_form
                .as_ref()
                .map_or_else(div, |draft| self.settings_downloads(draft, cx)),
            _ => div()
                .flex()
                .flex_col()
                .gap_5()
                .child(div().text_2xl().child("General"))
                .child(note("Make Block feel at home."))
                .child(self.settings_appearance(cx)),
        };
        let body = if self.preferences.reduce_motion {
            body.into_any_element()
        } else {
            body.with_animation(
                ("settings-pane", usize::from(page)),
                Animation::new(Duration::from_millis(220)).with_easing(|t| 1. - (1. - t).powi(3)),
                |element, progress| {
                    element
                        .relative()
                        .top(px((1. - progress) * 12.))
                        .opacity(progress)
                },
            )
            .into_any_element()
        };
        div().id("settings-page").size_full().min_h_0().flex().flex_col()
            .child(div().flex_1().min_h_0().flex()
                .child(div().w(px(220.)).flex_shrink_0().p_3().flex().flex_col().gap_2()
                    .border_r_1().border_color(rgb(self.palette.border))
                    .children([(0,"general","General"),(1,"game","Game defaults"),(2,"profiles","Profile mode"),(3,"downloads","Downloads & network")].map(|(index,id,label)| {
                        self.button(format!("settings-page-{id}"), if page == index { format!("● {label}") } else { label.into() }, UiAction::SettingsPage(index), cx)
                    })))
                .child(div().id("settings-content").flex_1().min_w_0().min_h_0().overflow_y_scroll().p_6().child(body)))
            .child(div().border_t_1().border_color(rgb(self.palette.border)).p_4().flex_shrink_0().flex().flex_col().gap_3()
                .child(note(self.status.clone()).id("settings-status").debug_selector(|| "settings-status".into()))
                .child(note("Appearance previews immediately. Save keeps changes; Cancel discards the draft. New game defaults preserve existing games."))
                .child(div().flex().justify_between().gap_2()
                    .child(self.button("reset-preferences", "Restore defaults", UiAction::ResetPreferences, cx))
                    .child(div().flex().gap_2()
                        .child(self.button("cancel-preferences", "Cancel", UiAction::Tab(Tab::Home), cx))
                        .child(self.button("save-preferences", "Save changes", UiAction::SavePreferences, cx)))))
    }

    fn settings_appearance(&self, cx: &Context<'_, Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_lg().child("Appearance"))
            .child(
                div().flex().gap_2().children(
                    [
                        ("system", "System", Appearance::System),
                        ("light", "Light", Appearance::Light),
                        ("dark", "Dark", Appearance::Dark),
                    ]
                    .map(|(id, label, appearance)| {
                        self.button(
                            format!("appearance-{id}"),
                            if self.preferences.appearance == appearance {
                                format!("● {label}")
                            } else {
                                label.into()
                            },
                            UiAction::SetAppearance(appearance),
                            cx,
                        )
                    }),
                ),
            )
            .child(self.button(
                "reduce-motion",
                if self.preferences.reduce_motion {
                    "Reduce motion: On"
                } else {
                    "Reduce motion: Off"
                },
                UiAction::ReduceMotion,
                cx,
            ))
    }

    fn settings_fields(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_lg().child("New game defaults"))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(field("Memory (MiB, 256–65536)", &draft.memory).flex_1())
                    .child(field("Java executable", &draft.java).flex_1()),
            )
            .child(field("JVM arguments", &draft.jvm))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(field("Window width", &draft.width).flex_1())
                    .child(field("Window height", &draft.height).flex_1()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "settings-fullscreen",
                        if draft.fullscreen {
                            "Fullscreen: On"
                        } else {
                            "Fullscreen: Off"
                        },
                        UiAction::SetFullscreen,
                        cx,
                    ))
                    .child(self.button(
                        "settings-snapshots",
                        if draft.show_snapshots {
                            "Snapshots: Shown"
                        } else {
                            "Snapshots: Hidden"
                        },
                        UiAction::ShowSnapshots,
                        cx,
                    )),
            )
    }

    fn settings_downloads(&self, draft: &SettingsDraft, cx: &Context<'_, Self>) -> gpui::Div {
        div().flex().flex_col().gap_3()
            .child(div().text_lg().child("Downloads"))
            .child(div().flex().gap_4()
                .child(field("Concurrent transfers (1–16)", &draft.concurrent).flex_1())
                .child(field("Additional retries (0–10)", &draft.retries).flex_1()))
            .child(field("Total object download rate (bytes/s)", &draft.rate))
            .child(note("Empty means unlimited. The limit is shared by active object transfers; index metadata is not throttled."))
            .child(div().flex().gap_2().children([(0, "system", "System"), (1, "direct", "Direct"), (2, "http", "HTTP / HTTPS")].map(|(mode,id,label)| {
                self.button(format!("settings-proxy-{id}"), if draft.proxy_mode == mode { format!("● {label}") } else { label.into() }, UiAction::ProxyMode(mode), cx)
            })))
            .children((draft.proxy_mode == 2).then(|| field("Proxy URL", &draft.proxy_url)))
            .child(note("HTTP and HTTPS proxies are supported. Enter a host and optional port without credentials."))
    }

    fn settings_profile_mode(&self, cx: &Context<'_, Self>) -> gpui::Div {
        div().flex().flex_col().gap_2().child(div().text_lg().child("Profile mode"))
            .child(if self.snapshot.profile_mode == ProfileMode::Separated {
                note("Separated profiles · several launch profiles can share an installation.").into_any_element()
            } else if self.shell.confirm_separation {
                div().flex().flex_col().gap_2()
                    .child(note("Switch to Separated profiles? Existing games and files stay in place. This conversion cannot be reversed."))
                    .child(self.button("confirm-separate", "Switch to Separated", UiAction::SeparateProfiles, cx))
                    .child(self.button("cancel-separate", "Keep Bundled", UiAction::ConfirmSeparate(false), cx)).into_any_element()
            } else {
                div().flex().flex_col().gap_2().child(note("Bundled profiles · one game, one profile. Recommended for beginners."))
                    .child(self.button("separate-profiles", "Switch to Separated…", UiAction::ConfirmSeparate(true), cx)).into_any_element()
            })
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
            let (view, cx) = cx.add_window_view(crate::tests::test_view);
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
            let status = cx.debug_bounds("settings-status");
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
        cx.run_until_parked();
        let bounds = cx.debug_bounds("save-preferences");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
    }
}
