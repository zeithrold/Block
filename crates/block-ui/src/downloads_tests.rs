//! Downloader events drive the product; no sample task names or progress clock.
use std::sync::{Arc, Mutex};

use block_launcher::DownloadEvent;
use gpui::{Modifiers, point, px, size};

use super::{DownloadGroup, DownloadTracker};
use crate::{
    UiAction,
    shell::{Drawer, Surface},
};

fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    settle(cx);
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "missing product control {selector}");
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.run_until_parked();
        crate::test_support::ensure_live_window(cx);
    }
    settle(cx);
}

fn tracker() -> DownloadTracker {
    let mut tracker = DownloadTracker::default();
    for index in 0..40 {
        let id = format!("{index:040x}");
        tracker.apply(DownloadEvent::Started {
            sha1: id.clone(),
            size: 100,
        });
        tracker.apply(DownloadEvent::Bytes {
            sha1: id.clone(),
            bytes: 25,
        });
        if index < 2 {
            tracker.apply(DownloadEvent::Complete {
                sha1: id,
                cached: index == 1,
            });
        }
    }
    tracker
}

#[test]
fn groups_partition_every_observed_transfer_and_preserve_transport_evidence() {
    let tracker = tracker();
    let counts = DownloadGroup::ALL.map(|group| {
        tracker
            .transfers()
            .filter(|file| group.includes(file))
            .count()
    });
    assert_eq!(counts, [38, 1, 1]);
    assert_eq!(
        counts.into_iter().sum::<usize>(),
        tracker.transfers().count()
    );
    assert_eq!(tracker.transport_bytes, 1000);
}

#[gpui::test]
fn download_details_show_all_files_and_back_restores_the_mouse_trigger(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.busy = true;
        view.downloads = Some(Arc::new(Mutex::new(tracker())));
        view.handle(UiAction::Drawer(Some(Drawer::Downloads)), window, cx);
        view
    });
    cx.simulate_resize(size(px(432.), px(768.)));
    settle(cx);

    click(cx, "download-group-active");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 2);
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DownloadFiles(DownloadGroup::Active))
        ));
        assert!(view.shell.downloads.files_scroll[0].max_offset().y > px(0.));
    });

    view.update(cx, |view, cx| {
        let scroll = &view.shell.downloads.files_scroll[0];
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        cx.notify();
    });
    settle(cx);
    let last = crate::test_support::control_bounds(
        cx,
        "download-file-0000000000000000000000000000000000000027",
    );
    let viewport = crate::test_support::control_bounds(cx, "download-files-scroll");
    assert!(
        last.zip(viewport)
            .is_some_and(|(last, viewport)| last.top() >= viewport.top() - px(1.)
                && last.bottom() <= viewport.bottom() + px(1.)),
        "last={last:?}; viewport={viewport:?}"
    );
    click(cx, "downloads-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.surfaces.layers().count()),
        1
    );

    click(cx, "download-expand");
    assert!(!view.read_with(cx, |view, _| view.shell.downloads.disclosure.is_open()));

    click(cx, "download-expand");
    view.update(cx, |view, cx| {
        let scroll = &view.shell.downloads.scroll;
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        cx.notify();
    });
    click(cx, "download-group-cached");
}
