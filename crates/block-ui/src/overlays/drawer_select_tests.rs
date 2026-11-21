//! Styled Select interactions through the production Drawer and its modal sheet.
#![expect(
    clippy::expect_used,
    reason = "An absent trigger, popup row, panel or scrim is a fixture failure."
)]

use gpui::{
    AnyElement, App, AppContext as _, Entity, FocusHandle, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString, TestAppContext,
    VisualTestContext, Window, div, px, size,
};
use gpui_component::{
    ActiveTheme as _, IndexPath,
    select::{Select, SelectItem, SelectState},
};

use crate::{
    Block, UiAction,
    shell::Drawer,
    test_support::{control_bounds, press, window},
};

const TRIGGER: &str = "drawer-select-trigger";
const FIRST_OPTION: &str = "drawer-select-first-option";

#[derive(Clone)]
struct OptionRow(usize);

impl SelectItem for OptionRow {
    type Value = usize;

    fn title(&self) -> SharedString {
        format!("Option {}", self.0 + 1).into()
    }

    fn value(&self) -> &usize {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .debug_selector(|| {
                if self.0 == 0 {
                    FIRST_OPTION.to_owned()
                } else {
                    format!("drawer-select-option-{}", self.0)
                }
            })
            .child(self.title())
    }
}

pub(super) struct SelectBody(Entity<SelectState<Vec<OptionRow>>>);

impl Global for SelectBody {}

impl SelectBody {
    pub(super) fn render(&self) -> AnyElement {
        div()
            .debug_selector(|| TRIGGER.to_owned())
            .child(Select::new(&self.0).menu_max_h(px(440.)))
            .into_any_element()
    }
}

struct Fixture {
    view: Entity<Block>,
    select: Entity<SelectState<Vec<OptionRow>>>,
    invoking_focus: FocusHandle,
    trigger_focus: FocusHandle,
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    let (view, cx) = window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(432.), px(1024.)));
    cx.run_until_parked();
    let (select, invoking_focus, trigger_focus) = cx.update(|window, cx| {
        let invoking_focus = window.focused(cx).expect("focus before opening the Drawer");
        let select = cx.new(|cx| {
            SelectState::new(
                (0..24).map(OptionRow).collect::<Vec<_>>(),
                Some(IndexPath::new(1)),
                window,
                cx,
            )
        });
        let trigger_focus = select.focus_handle(cx);
        cx.set_global(SelectBody(select.clone()));
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Downloads)), window, cx);
        });
        (select, invoking_focus, trigger_focus)
    });
    let trigger = control_bounds(cx, TRIGGER).expect("Select trigger in the live Drawer");
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    (
        Fixture {
            view,
            select,
            invoking_focus,
            trigger_focus,
        },
        cx,
    )
}

#[gpui::test]
fn select_option_above_the_drawer_paints_over_the_backdrop_and_accepts_pointer(
    cx: &mut TestAppContext,
) {
    let (fixture, cx) = fixture(cx);
    let panel = control_bounds(cx, "block-drawer").expect("live Drawer panel");
    let backdrop = control_bounds(cx, "block-drawer-backdrop").expect("live Drawer backdrop");
    let option = control_bounds(cx, FIRST_OPTION).expect("first option in the open popup");
    let position = option.center();
    assert!(position.y < panel.top() && backdrop.contains(&position));
    cx.update(|window, cx| {
        let scale = window.scale_factor();
        let position = position.scale(scale);
        let quads = window.painted_quads();
        let scrim = quads
            .iter()
            .find(|quad| {
                quad.bounds == backdrop.scale(scale)
                    && quad.background.as_solid().is_some_and(|color| {
                        color.l.abs() < f32::EPSILON && (color.a - 0.1).abs() < 0.001
                    })
            })
            .expect("painted Drawer backdrop");
        assert!(
            quads.iter().any(|quad| {
                quad.order > scrim.order
                    && quad.bounds.contains(&position)
                    && quad.content_mask.bounds.contains(&position)
                    && quad.background.as_solid() == Some(cx.theme().popover)
            }),
            "popup must paint at the option outside the panel, above the scrim"
        );
    });

    cx.simulate_click(position, Default::default());
    cx.run_until_parked();
    assert_eq!(
        fixture
            .select
            .read_with(cx, |select, _| select.selected_value().copied()),
        Some(0),
        "the click above the panel must reach the real Select option"
    );
    assert!(
        fixture
            .view
            .read_with(cx, |view, _| view.shell.surfaces.is_open())
    );
}

#[gpui::test]
fn escape_closes_the_select_before_the_drawer_and_restores_both_triggers(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx);
    assert!(control_bounds(cx, FIRST_OPTION).is_some());
    assert_ne!(
        cx.update(|window, app| window.focused(app)),
        Some(fixture.trigger_focus.clone()),
        "the open popup must have moved focus off the trigger"
    );

    press(cx, "escape");
    assert!(control_bounds(cx, FIRST_OPTION).is_none());
    assert!(control_bounds(cx, "block-drawer").is_some());
    assert_eq!(
        cx.update(|window, app| window.focused(app)),
        Some(fixture.trigger_focus)
    );

    press(cx, "escape");
    assert!(control_bounds(cx, "block-drawer").is_none());
    assert!(
        !fixture
            .view
            .read_with(cx, |view, _| view.shell.surfaces.is_open())
    );
    assert_eq!(
        cx.update(|window, app| window.focused(app)),
        Some(fixture.invoking_focus)
    );
}
