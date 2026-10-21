//! First-run setup uses the management aspect ratio and persists only on completion.
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px, rgb,
};

use super::{Appearance, Block, Tab, UiAction, note};

impl Block {
    pub(super) fn onboarding(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let step = self.shell.onboarding_step;
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(240.))
                    .flex_shrink_0()
                    .p_6()
                    .bg(rgb(self.palette.surface))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(super::shell::brand(48.))
                    .child(div().text_xl().child("Block"))
                    .child(note("Let's get you started"))
                    .children(
                        ["Welcome", "Appearance", "Next steps"]
                            .iter()
                            .enumerate()
                            .map(|(index, label)| {
                                div().py_3().child(format!(
                                    "{}  {label}",
                                    if index == step { "●" } else { "○" }
                                ))
                            }),
                    )
                    .child(div().flex_1())
                    .child(note("block.ztd.one")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .id("onboarding-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.onboarding_content(cx)),
                    )
                    .child(note(self.status.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(if step > 0 {
                                self.button("welcome-back", "Back", UiAction::OnboardingBack, cx)
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            })
                            .child(self.button(
                                "welcome-next",
                                if step == 2 {
                                    "Start using Block"
                                } else {
                                    "Continue"
                                },
                                UiAction::OnboardingNext,
                                cx,
                            )),
                    ),
            )
    }

    fn onboarding_content(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let step = self.shell.onboarding_step;
        let title = [
            "Welcome to Block",
            "Make yourself at home",
            "You're ready to begin",
        ];
        let description = [
            "Install Minecraft, choose a game, and start your next adventure.",
            "Choose how Block looks. You can change these settings at any time.",
            "Your library starts empty. Add your own games and sign in when you're ready to play.",
        ];
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_2xl().child(title[step]))
            .child(note(description[step]));
        match step {
            0 => {
                body = body.child(super::shell::brand(112.))
                    .child(div().text_lg().child("Your games in one place"))
                    .child(note("Block installs Java, libraries and game files, and keeps launch settings with each game."))
                    .child(div().text_lg().child("Your worlds stay yours"))
                    .child(note("Deleting a game from Block keeps its worlds and files on disk. Microsoft credentials stay in your system's secure storage."));
            }
            1 => {
                body = body.child(div().flex().gap_3().children([
                    ("welcome-system", "Use system theme", Appearance::System),
                    ("welcome-light", "Light", Appearance::Light),
                    ("welcome-dark", "Dark", Appearance::Dark),
                ].map(|(id, label, appearance)| self.button(id, label, UiAction::SetAppearance(appearance), cx))))
                    .child(self.button("welcome-motion", if self.preferences.reduce_motion { "Reduced motion: On" } else { "Reduced motion: Off" }, UiAction::ReduceMotion, cx))
                    .child(note("Reduce motion turns off interface animations. It does not affect the game."));
            }
            _ => {
                body = body.child(div().text_lg().child("1. Add a game"))
                    .child(note("Choose a Minecraft version and review the installation before downloading."))
                    .child(div().text_lg().child("2. Sign in with Microsoft"))
                    .child(note("Use the system browser to sign in. A Minecraft Java Edition license is required for online play."))
                    .child(div().text_lg().child("3. Launch"))
                    .child(note("Pick a game from Home and press Launch. Block checks its files and account before starting."));
            }
        }
        body
    }

    pub(super) fn advance_onboarding(&mut self, window: &mut gpui::Window) {
        if self.tab != Tab::Onboarding {
            return;
        }
        if self.shell.onboarding_step < 2 {
            self.shell.onboarding_step += 1;
        } else {
            let mut preferences = self.preferences.clone();
            preferences.onboarding_complete = true;
            let Some(engine) = &self.engine else {
                self.report(
                    "Setup could not be saved. Check Block's data directory and restart.".into(),
                );
                return;
            };
            if let Err(error) = preferences.save(&engine.root) {
                self.report(format!("Could not finish setup: {error}"));
                return;
            }
            self.saved_preferences = preferences.clone();
            self.preferences = preferences;
            self.tab = Tab::Home;
            self.status = "Welcome to Block · create your first game".into();
            window.resize(gpui::size(px(432.), px(768.)));
        }
        self.shell.revision = self.shell.revision.wrapping_add(1);
    }
}
