use super::{
    TargetDraft, VERSION, draft_version, open, press, row_text, select, settle, settle_tab,
    version, wizard,
};

#[gpui::test]
fn open_target_catalog_refresh_preserves_values_across_a_reorder(cx: &mut gpui::TestAppContext) {
    let (view, cx) = wizard(
        cx,
        TargetDraft {
            version: Some(version(1)),
            ..TargetDraft::default()
        },
    );
    view.update(cx, |view, cx| {
        view.versions = [0, 1, 2].map(version).into();
        cx.notify();
    });
    settle(cx);
    open(&view, cx, VERSION);
    assert_eq!(row_text(cx, VERSION), [version(0), version(1), version(2)]);
    press(cx, "down");
    settle_tab(cx);
    view.update(cx, |view, cx| {
        view.versions = [2, 3, 0, 1].map(version).into();
        cx.notify();
    });
    settle_tab(cx);
    assert_eq!(select(cx, VERSION).expanded(), Some(true));
    assert_eq!(select(cx, VERSION).value(), Some(version(1).as_str()));
    assert_eq!(draft_version(&view, cx), Some(version(1)));
    assert_eq!(
        row_text(cx, VERSION),
        [version(2), version(3), version(0), version(1)]
    );
    press(cx, "enter");
    settle_tab(cx);
    assert_eq!(
        draft_version(&view, cx),
        Some(version(2)),
        "confirm uses the inspected value rather than its former index"
    );
}
