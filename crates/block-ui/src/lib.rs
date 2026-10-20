//! Native launcher UI backed by the same recipe/plan/executor engine as the CLI.

mod engine;

use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

use block_launcher::{
    CancellationToken,
    recipe::{Compatibility, LaunchRecipe, RecipeSource, RuntimeSelection},
    session::OfflineSession,
};
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    PathPromptOptions, Render, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, px, rgb, uniform_list,
};

use engine::{Engine, Message};
use tokio::sync::mpsc::Receiver;

/// Optional startup recipe and an explicit request to launch it after validation.
#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub recipe: Option<std::path::PathBuf>,
    pub launch: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher)]
struct Launch;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher)]
struct Stop;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher)]
struct NextVersion;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher)]
struct PreviousVersion;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher)]
struct OpenRecipe;

#[derive(Debug)]
struct VersionRow {
    id: String,
    kind: String,
}

/// A native recipe-driven launcher. Blocking work stays on the engine runtime.
#[derive(Debug)]
pub struct Block {
    recipe: LaunchRecipe,
    versions: Vec<VersionRow>,
    selected: usize,
    engine: Option<Engine>,
    messages: Option<Receiver<Message>>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    status: String,
    completed: usize,
    busy: bool,
    cancel: CancellationToken,
    logs: VecDeque<String>,
    command: Option<String>,
    show_command: bool,
    launch_on_load: bool,
}

impl Drop for Block {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Block {
    #[must_use]
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>, options: LaunchOptions) -> Self {
        cx.bind_keys([
            KeyBinding::new("enter", Launch, Some("Launcher")),
            KeyBinding::new("escape", Stop, Some("Launcher")),
            KeyBinding::new("down", NextVersion, Some("Launcher")),
            KeyBinding::new("up", PreviousVersion, Some("Launcher")),
            KeyBinding::new("cmd-o", OpenRecipe, Some("Launcher")),
            KeyBinding::new("ctrl-o", OpenRecipe, Some("Launcher")),
        ]);
        let focus = cx.focus_handle();
        focus.focus(window);
        let (engine, messages, status) = match Engine::new() {
            Ok((engine, messages)) => {
                engine.load_versions();
                (
                    Some(engine),
                    Some(messages),
                    "Ready · loading version catalog".into(),
                )
            }
            Err(error) => (None, None, format!("Engine unavailable: {error:#}")),
        };
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if view.update(cx, Self::poll).is_err() {
                    break;
                }
            }
        })
        .detach();
        let has_startup_recipe = options.recipe.is_some();
        if let (Some(engine), Some(path)) = (&engine, options.recipe) {
            engine.load_recipe(path);
        }
        let mut view = Self {
            recipe: default_recipe(),
            versions: ["1.20.6", "1.20.1", "26.3"]
                .into_iter()
                .map(|id| VersionRow {
                    id: id.into(),
                    kind: "release".into(),
                })
                .collect(),
            selected: 0,
            engine,
            messages,
            focus,
            scroll: UniformListScrollHandle::new(),
            status,
            completed: 0,
            busy: false,
            cancel: CancellationToken::new(),
            logs: VecDeque::new(),
            command: None,
            show_command: false,
            launch_on_load: has_startup_recipe && options.launch,
        };
        if options.launch && !has_startup_recipe {
            view.launch(cx);
        }
        view
    }

    fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let mut messages = Vec::new();
        if let Some(receiver) = &mut self.messages {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }
        if messages.is_empty() {
            return;
        }
        for message in messages {
            let should_launch = matches!(&message, Message::Recipe(_)) && self.launch_on_load;
            self.apply_message(message);
            if should_launch {
                self.launch_on_load = false;
                self.launch(cx);
            }
        }
        cx.notify();
    }

    fn apply_message(&mut self, message: Message) {
        match message {
            Message::Versions(versions) => {
                self.versions = versions
                    .into_iter()
                    .map(|entry| VersionRow {
                        id: entry.id,
                        kind: format!("{:?}", entry.kind).to_lowercase(),
                    })
                    .collect();
                if let RecipeSource::Mojang { version } = &self.recipe.source {
                    self.selected = self
                        .versions
                        .iter()
                        .position(|row| row.id == *version)
                        .unwrap_or(0);
                    self.scroll
                        .scroll_to_item_strict(self.selected, ScrollStrategy::Center);
                }
                if !self.busy {
                    self.status = "Ready".into();
                }
            }
            Message::Recipe(recipe) => {
                self.recipe = recipe;
                self.status = "Recipe loaded · ready".into();
                self.command = None;
                if let RecipeSource::Mojang { version } = &self.recipe.source {
                    self.selected = self
                        .versions
                        .iter()
                        .position(|row| row.id == *version)
                        .unwrap_or(0);
                    self.scroll
                        .scroll_to_item_strict(self.selected, ScrollStrategy::Center);
                }
            }
            Message::Status(status) => self.status = status,
            Message::Progress(count) => self.completed = count,
            Message::Command(command) => self.command = Some(command),
            Message::Log(line) => {
                if self.logs.len() == 200 {
                    self.logs.pop_front();
                }
                self.logs.push_back(line);
            }
            Message::Finished(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(_) if self.cancel.is_cancelled() => "Stopped".into(),
                    Ok(code) => format!("Minecraft exited · {code:?}"),
                    Err(_) if self.cancel.is_cancelled() => "Cancelled".into(),
                    Err(error) => {
                        self.logs.push_back(error.clone());
                        format!("Launch failed: {error}")
                    }
                };
            }
        }
    }

    fn launch(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(engine) = &self.engine {
            self.busy = true;
            self.completed = 0;
            self.command = None;
            self.show_command = false;
            self.logs.clear();
            self.status = "Resolving launch recipe".into();
            self.cancel = CancellationToken::new();
            engine.launch(self.recipe.clone(), self.cancel.clone());
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            self.cancel.cancel();
            self.status = "Stopping".into();
            cx.notify();
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(row) = self.versions.get(index) {
            self.recipe.source = RecipeSource::Mojang {
                version: row.id.clone(),
            };
            self.recipe.instance = format!(
                "vanilla-{}",
                row.id
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect::<String>()
            );
            self.selected = index;
            self.command = None;
            self.status = "Ready".into();
            self.scroll
                .scroll_to_item_strict(index, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn open_recipe(&self, cx: &Context<'_, Self>) {
        if self.busy {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a Block launch recipe (.json)".into()),
        });
        cx.spawn(async move |view, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = view.update(cx, |view, cx| {
                        if let Some(engine) = &view.engine {
                            engine.load_recipe(path);
                            view.status = "Reading launch recipe".into();
                            cx.notify();
                        }
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = view.update(cx, |view, cx| {
                    view.status = format!("File picker failed: {error}");
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn title(&self) -> String {
        match &self.recipe.source {
            RecipeSource::Mojang { version } => format!("Minecraft {version}"),
            RecipeSource::LocalManifest { .. } => "Local manifest recipe".into(),
        }
    }

    fn header() -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(rgb(0x002c_3340))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xl().child("Block"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x0099_a6bc))
                            .child("Minecraft Java launcher"),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x0099_a6bc))
                    .child("OFFLINE MODE"),
            )
    }

    fn version_list(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .w(px(224.0))
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(0x002c_3340))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .text_xs()
                    .text_color(rgb(0x0099_a6bc))
                    .child(format!("VERSIONS · {}", self.versions.len())),
            )
            .child(
                uniform_list(
                    "versions",
                    self.versions.len(),
                    cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                        range.map(|index| view.version_row(index, cx)).collect()
                    }),
                )
                .flex_1()
                .track_scroll(self.scroll.clone()),
            )
    }

    fn version_row(&self, index: usize, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let row = &self.versions[index];
        div()
            .id(index)
            .h(px(60.0))
            .flex()
            .flex_col()
            .px_4()
            .py_2()
            .gap_1()
            .cursor_pointer()
            .bg(rgb(if index == self.selected {
                0x0028_3c39
            } else {
                0x0017_1d27
            }))
            .hover(|style| style.bg(rgb(0x0027_303e)))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.focus.focus(window);
                view.select(index, cx);
            }))
            .child(div().text_sm().child(row.id.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x0099_a6bc))
                    .child(row.kind.clone()),
            )
    }

    fn controls(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let OfflineSession::Offline { username } = &self.recipe.session;
        let runtime_label = match self.recipe.runtime {
            RuntimeSelection::Managed => "Automatic Java",
            RuntimeSelection::Local { .. } => "Custom Java",
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_2xl().child(self.title()))
                    .child(div().text_sm().text_color(rgb(0x0099_a6bc)).child(format!(
                        "{username} · {} MiB heap · {runtime_label}",
                        self.recipe.memory_mib
                    ))),
            )
            .child(
                div()
                    .p_4()
                    .rounded_md()
                    .bg(rgb(0x001e_2734))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child(self.status.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x0099_a6bc))
                            .child(format!("{} resource objects verified", self.completed)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("launch")
                            .px_4()
                            .py_3()
                            .rounded_md()
                            .bg(rgb(if self.busy { 0x0036_4b49 } else { 0x003b_8c72 }))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| view.launch(cx)))
                            .child(if self.busy {
                                "Preparing / running"
                            } else {
                                "Launch offline"
                            }),
                    )
                    .child(
                        div()
                            .id("stop")
                            .px_4()
                            .py_3()
                            .rounded_md()
                            .bg(rgb(0x0034_3d4c))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| view.stop(cx)))
                            .child("Stop"),
                    )
                    .child(
                        div()
                            .id("open-recipe")
                            .px_4()
                            .py_3()
                            .rounded_md()
                            .bg(rgb(0x0034_3d4c))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| view.open_recipe(cx)))
                            .child("Open recipe…"),
                    ),
            )
    }

    fn output(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let lines: Vec<String> = if self.show_command {
            self.command
                .as_deref()
                .unwrap_or("The complete Java command will appear after preparation.")
                .lines()
                .map(str::to_owned)
                .collect()
        } else if self.logs.is_empty() {
            vec![
                "Choose a version or open a recipe, then launch.".into(),
                "Downloads, native libraries and Java run in the background.".into(),
            ]
        } else {
            self.logs.iter().cloned().collect()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(0x0099_a6bc))
                    .child(if self.show_command {
                        "JAVA COMMAND · REDACTED PREVIEW"
                    } else {
                        "GAME OUTPUT"
                    })
                    .child(
                        div()
                            .id("toggle-command")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_command = !view.show_command;
                                cx.notify();
                            }))
                            .child(if self.show_command {
                                "Show output"
                            } else {
                                "Show command"
                            }),
                    ),
            )
            .child(
                div()
                    .id("output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .rounded_md()
                    .bg(rgb(0x000e_141d))
                    .text_xs()
                    .children(lines.into_iter().map(|line| div().mb_1().child(line))),
            )
    }
}

impl Render for Block {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .key_context("Launcher")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0017_1d27))
            .text_color(rgb(0x00eb_f0f8))
            .on_action(cx.listener(|view, _: &Launch, _, cx| view.launch(cx)))
            .on_action(cx.listener(|view, _: &Stop, _, cx| view.stop(cx)))
            .on_action(cx.listener(|view, _: &NextVersion, _, cx| {
                view.select(
                    (view.selected + 1).min(view.versions.len().saturating_sub(1)),
                    cx,
                );
            }))
            .on_action(cx.listener(|view, _: &PreviousVersion, _, cx| {
                view.select(view.selected.saturating_sub(1), cx);
            }))
            .on_action(cx.listener(|view, _: &OpenRecipe, _, cx| view.open_recipe(cx)))
            .child(Self::header())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.version_list(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .p_6()
                            .gap_6()
                            .child(self.controls(cx))
                            .child(self.output(cx)),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .py_3()
                    .text_xs()
                    .text_color(rgb(0x0099_a6bc))
                    .border_t_1()
                    .border_color(rgb(0x002c_3340))
                    .child("↑ ↓ Choose version   Enter Launch   Esc Stop   Ctrl/Cmd+O Open recipe"),
            )
    }
}

fn default_recipe() -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: "vanilla-1_20_6".into(),
        source: RecipeSource::Mojang {
            version: "1.20.6".into(),
        },
        session: OfflineSession::Offline {
            username: "Player".into(),
        },
        runtime: RuntimeSelection::Managed,
        target: None,
        compatibility: Compatibility::Official,
        memory_mib: 2048,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    }
}
