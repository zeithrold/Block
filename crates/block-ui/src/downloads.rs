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
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Active => "Pending verification",
            Self::Verified => "Verified downloads",
            Self::Cached => "Verified cache hits",
        }
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

    pub(super) fn render_downloads(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::Div {
        let mut page = div()
            .h((window.viewport_size().height - px(200.)).clamp(px(160.), px(360.)))
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child("Downloads"))
            .child(note(self.status.clone()));
        let Some(tracker) = self.download_snapshot() else {
            return page.child(patterns::empty_state("No observed transfers", if self.busy { "The operation is preparing resources. Files appear when the downloader reports them." } else { "Start an installation or repair to track real downloads." }, self.palette));
        };
        let count = tracker.transfers().count();
        let completed = tracker.verified();
        let trigger = motion::disclosure_trigger(
            "download-expand",
            "Observed resources",
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
        let content = div().flex().flex_col().gap_2().child(trigger)
            .child(components::progress(ProgressState::from_counts(completed as u64, (count > 0).then_some(count as u64)), self.palette))
            .child(note(format!("{completed} of {count} observed resources verified. {} bytes received from the network, including retries. The overall plan total is unknown.", tracker.transport_bytes)))
            .child(motion::disclosure("download-groups", &self.shell.downloads.disclosure, self.motion_preferences(), groups.into_any_element(), window, cx));
        page = page.child(
            super::scroll_area::viewport(
                div()
                    .id("download-tasks-scroll")
                    .debug_selector(|| "download-tasks-scroll".into())
                    .min_h_0()
                    .flex_1(),
                &self.shell.downloads.scroll,
                "Download tasks",
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
            group.label(),
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
            format!("{} files", files.len()),
            self.palette,
            if completed > 0 {
                BadgeVariant::Success
            } else {
                BadgeVariant::Secondary
            },
        ))
        .child(Icon::ChevronRight.render(14., self.palette.muted))
    }

    pub(super) fn render_download_files(
        &self,
        group: DownloadGroup,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
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
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                self.button(
                    "downloads-back",
                    "Back to tasks",
                    UiAction::Drawer(None),
                    cx,
                )
                .child(Icon::ArrowLeft.render(14., self.palette.muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .whitespace_normal()
                    .child(group.label()),
            );
        let rows = div().flex().flex_col().gap_2()
            .children(files.iter().map(|file| self.download_file_row(file)))
            .children(files.is_empty().then(|| patterns::empty_state("No resources in this group", "Transfer updates can move resources between groups. Back returns to the task list.", self.palette)));
        div().h((window.viewport_size().height - px(200.)).clamp(px(160.), px(360.)))
            .min_w_0().flex().flex_col().gap_3().child(header)
            .child(note(format!("{} observed resources. Resource IDs are SHA-1 hashes; the downloader does not report file names.", files.len())))
            .child(super::scroll_area::viewport(div().id("download-files-scroll").debug_selector(|| "download-files-scroll".into()).min_h_0().flex_1(), &self.shell.downloads.files_scroll[group.index()], "Download files", self.palette, false).child(rows))
    }

    fn download_file_row(&self, file: &Transfer) -> impl gpui::IntoElement + use<> {
        let status = if file.cached {
            "Verified cache hit"
        } else if file.complete {
            "Downloaded and verified"
        } else if file.retries > 0 {
            "Not verified after retry"
        } else {
            "Not yet verified"
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
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_xs()
                            .truncate()
                            .child(format!("Resource {}", file.id)),
                    ),
            )
            .child(components::progress(progress, self.palette))
            .child(note(format!(
                "{} / {} bytes · {status}{}",
                file.received,
                file.size
                    .map_or_else(|| "unknown".into(), |size| size.to_string()),
                if file.retries > 0 {
                    format!(" · retry {}", file.retries)
                } else {
                    String::new()
                }
            )))
    }
}
