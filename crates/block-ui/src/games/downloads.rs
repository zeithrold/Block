//! Real observed transfers, grouped without inventing a download plan or names.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Downloads are a private product surface."
)]

use gpui::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, px, rgb,
};

use super::{
    Block, UiAction,
    components::{self, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    motion::{self, Disclosure},
    note, patterns,
    tasks::{DownloadTracker, Transfer},
};

#[cfg(test)]
#[path = "downloads_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DownloadGroup {
    Active,
    Verified,
    Cached,
}

impl DownloadGroup {
    const ALL: [Self; 3] = [Self::Active, Self::Verified, Self::Cached];
    const fn index(self) -> usize {
        match self {
            Self::Active => 0,
            Self::Verified => 1,
            Self::Cached => 2,
        }
    }
    const fn slug(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Verified => "verified",
            Self::Cached => "cached",
        }
    }
    pub(super) fn label(self, language: crate::Language) -> String {
        tr!(
            language,
            match self {
                Self::Active => "downloads.pending",
                Self::Verified => "downloads.verified",
                Self::Cached => "downloads.cached",
            }
        )
    }
    const fn includes(self, transfer: &Transfer) -> bool {
        match self {
            Self::Active => !transfer.complete,
            Self::Verified => transfer.complete && !transfer.cached,
            Self::Cached => transfer.complete && transfer.cached,
        }
    }
}

#[derive(Debug)]
pub(super) struct State {
    disclosure: Disclosure,
    pub scroll: ScrollHandle,
    pub files_scroll: [ScrollHandle; 3],
}

impl Default for State {
    fn default() -> Self {
        Self {
            disclosure: Disclosure::new(true),
            scroll: ScrollHandle::new(),
            files_scroll: std::array::from_fn(|_| ScrollHandle::new()),
        }
    }
}

impl State {
    pub(super) fn snapshot(&self) -> Self {
        let clone_scroll = |source: &ScrollHandle| {
            let scroll = ScrollHandle::new();
            scroll.set_offset(source.offset());
            scroll
        };
        Self {
            disclosure: self.disclosure.clone(),
            scroll: clone_scroll(&self.scroll),
            files_scroll: std::array::from_fn(|index| clone_scroll(&self.files_scroll[index])),
        }
    }
}

impl Block {
    pub(super) fn expand_downloads(&mut self, cx: &Context<'_, Self>) {
        let preferences = self.motion_preferences();
        let disclosure = &mut self.shell.downloads.disclosure;
        disclosure.set_open(
            !disclosure.is_open(),
            cx.background_executor().now(),
            preferences,
        );
    }

    fn download_snapshot(&self) -> Option<DownloadTracker> {
        self.downloads.as_ref().map(|tracker| {
            tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
    }

    /// Download task summary without a surface-local heading: the owning sheet
    /// supplies its own title and description.
    pub(super) fn render_downloads_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let language = self.language();
        let mut page = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(note(self.status.text(self.language())));
        let Some(tracker) = self.download_snapshot() else {
            return page.child(patterns::empty_state(
                tr!(language, "downloads.empty"),
                tr!(
                    language,
                    if self.busy {
                        "downloads.preparing"
                    } else {
                        "downloads.start"
                    }
                ),
                self.palette,
            ));
        };
        let count = tracker.transfers().count();
        let completed = tracker.verified();
        let trigger = motion::disclosure_trigger(
            "download-expand",
            tr!(language, "downloads.observed"),
            &self.shell.downloads.disclosure,
            self.palette,
            self.motion_preferences(),
            cx,
            cx.listener(|view, (): &(), _, cx| {
                view.expand_downloads(cx);
                cx.notify();
            }),
        )
        .h_auto()
        .min_w_0()
        .p_2();
        let groups = div()
            .flex()
            .flex_col()
            .gap_1()
            .border_l_1()
            .border_color(rgb(self.palette.border))
            .pl_2()
            .children(DownloadGroup::ALL.map(|group| self.download_group_row(group, &tracker, cx)));
        let content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(trigger)
            .child(components::progress(
                ProgressState::from_counts(completed as u64, (count > 0).then_some(count as u64)),
                self.palette,
                tr!(language, "common.in_progress"),
            ))
            .child(note(tr!(
                language,
                "downloads.summary",
                completed = completed,
                count = count,
                bytes = tracker.transport_bytes
            )))
            .child(motion::disclosure(
                "download-groups",
                &self.shell.downloads.disclosure,
                self.motion_preferences(),
                groups.into_any_element(),
                window,
                cx,
            ));
        page = page.child(
            super::scroll_area::viewport(
                div()
                    .id("download-tasks-scroll")
                    .debug_selector(|| "download-tasks-scroll".into())
                    .min_h_0()
                    .flex_1(),
                &self.shell.downloads.scroll,
                "downloads.tasks",
                self.palette,
                false,
            )
            .child(content),
        );
        page
    }

    fn download_group_row(
        &self,
        group: DownloadGroup,
        tracker: &DownloadTracker,
        cx: &Context<'_, Self>,
    ) -> impl gpui::IntoElement + use<> {
        let files: Vec<_> = tracker
            .transfers()
            .filter(|file| group.includes(file))
            .collect();
        let completed = files.iter().filter(|file| file.complete).count();
        components::button(
            format!("download-group-{}", group.slug()),
            group.label(self.language()),
            self.palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            cx.listener(move |view, (): &(), window, cx| {
                view.handle(UiAction::DownloadFiles(group), window, cx);
            }),
        )
        .w_full()
        .h_auto()
        .min_h(px(64.))
        .justify_start()
        .p_2()
        .child(div().flex_1())
        .child(components::badge(
            tr!(
                self.language(),
                if files.len() == 1 {
                    "downloads.one_file"
                } else {
                    "downloads.files"
                },
                count = files.len()
            ),
            self.palette,
            if completed > 0 {
                BadgeVariant::Success
            } else {
                BadgeVariant::Secondary
            },
        ))
        .child(Icon::ChevronRight.render(14., self.palette.muted))
    }

    /// File-level detail for one download group, without a surface-local
    /// heading or back control: the owning dialog supplies those.
    pub(super) fn render_download_files_body(&self, group: DownloadGroup) -> gpui::Div {
        let language = self.language();
        let files: Vec<_> = self
            .download_snapshot()
            .into_iter()
            .flat_map(|tracker| {
                tracker
                    .transfers()
                    .filter(|file| group.includes(file))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        let rows = div()
            .flex()
            .flex_col()
            .gap_2()
            .children(files.iter().map(|file| self.download_file_row(file)))
            .children(files.is_empty().then(|| {
                patterns::empty_state(
                    tr!(language, "downloads.no_group_resources"),
                    tr!(language, "downloads.moving_resources"),
                    self.palette,
                )
            }));
        div()
            .h(px(360.))
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(note(tr!(
                language,
                "downloads.resources_summary",
                count = files.len()
            )))
            .child(
                super::scroll_area::viewport(
                    div()
                        .id("download-files-scroll")
                        .debug_selector(|| "download-files-scroll".into())
                        .min_h_0()
                        .flex_1(),
                    &self.shell.downloads.files_scroll[group.index()],
                    "downloads.files",
                    self.palette,
                    false,
                )
                .child(rows),
            )
    }

    fn download_file_row(&self, file: &Transfer) -> impl gpui::IntoElement + use<> {
        let language = self.language();
        let status = tr!(
            language,
            if file.cached {
                "downloads.cache_hit"
            } else if file.complete {
                "downloads.downloaded"
            } else if file.retries > 0 {
                "downloads.retry_unverified"
            } else {
                "downloads.not_verified"
            }
        );
        let total = file.size.map_or_else(
            || tr!(language, "downloads.unknown"),
            |size| size.to_string(),
        );
        let progress_text = if file.retries > 0 {
            tr!(
                language,
                "downloads.progress_retry",
                received = file.received,
                total = total,
                status = status,
                retry = file.retries
            )
        } else {
            tr!(
                language,
                "downloads.progress",
                received = file.received,
                total = total,
                status = status
            )
        };
        let progress = if file.complete {
            ProgressState::from_counts(1, Some(1))
        } else {
            ProgressState::from_counts(file.received, file.size)
        };

        div()
            .id(gpui::SharedString::from(format!(
                "download-file-{}",
                file.id
            )))
            .debug_selector({
                let id = format!("download-file-{}", file.id);
                move || id
            })
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(self.palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        if file.complete {
                            Icon::Check
                        } else {
                            Icon::Download
                        }
                        .render(14., self.palette.muted),
                    )
                    .child(div().min_w_0().flex_1().text_xs().truncate().child(tr!(
                        language,
                        "downloads.resource",
                        id = file.id
                    ))),
            )
            .child(components::progress(
                progress,
                self.palette,
                tr!(language, "common.in_progress"),
            ))
            .child(note(progress_text))
    }
}
