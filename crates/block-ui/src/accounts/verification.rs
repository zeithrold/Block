//! Grouped Microsoft verification progress, driven by observed auth stages.

use block_auth::SignInStage;
use gpui::{
    InteractiveElement as _, IntoElement as _, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, div,
};

use super::{AuthWizard, Block, Language, components, note, wait_ring::WaitRing};
use crate::design::Icon;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepStatus {
    Complete,
    Active,
    Pending,
}

#[derive(Clone, Copy)]
enum Step {
    Authorization,
    Ownership,
    Profile,
}

impl Step {
    const ALL: [Self; 3] = [Self::Authorization, Self::Ownership, Self::Profile];

    const fn status(self, stage: Option<SignInStage>) -> StepStatus {
        use SignInStage as Stage;
        use StepStatus::{Active, Complete, Pending};
        match stage {
            // Persistence belongs to the separately confirmed Saving phase.
            None
            | Some(
                Stage::SecureStorage
                | Stage::DeviceAuthorization
                | Stage::BrowserAuthorization
                | Stage::SaveCredential
                | Stage::SaveMetadata,
            ) => Pending,
            Some(
                Stage::MicrosoftAuthorization
                | Stage::XboxAuthentication
                | Stage::XboxAuthorization
                | Stage::MinecraftAuthentication,
            ) => match self {
                Self::Authorization => Active,
                Self::Ownership | Self::Profile => Pending,
            },
            Some(Stage::Ownership) => match self {
                Self::Authorization => Complete,
                Self::Ownership => Active,
                Self::Profile => Pending,
            },
            Some(Stage::PlayerProfile) => match self {
                Self::Authorization | Self::Ownership => Complete,
                Self::Profile => Active,
            },
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Authorization => "accounts.auth.waiting.boundary.microsoft_xbox",
            Self::Ownership => "accounts.auth.waiting.boundary.ownership",
            Self::Profile => "accounts.auth.waiting.boundary.profile",
        }
    }
}

pub(super) fn step_statuses(stage: Option<SignInStage>) -> [StepStatus; 3] {
    Step::ALL.map(|step| step.status(stage))
}

impl Block {
    /// Preparation precedes every grouped verification boundary.
    pub(super) fn preparing_step_rows(
        &self,
        language: Language,
    ) -> impl Iterator<Item = gpui::Div> + use<> {
        let muted = self.palette.muted;
        Step::ALL.into_iter().map(move |step| {
            let label = tr!(language, step.label());
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::Info.render(14., muted))
                .child(note(tr!(
                    language,
                    "accounts.auth.waiting.pending",
                    label = label
                )))
        })
    }

    pub(super) fn microsoft_verification(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let animate = !self.visual_only && !self.preferences.reduce_motion;
        components::card(self.palette).children(
            Step::ALL
                .into_iter()
                .zip(step_statuses(state.stage))
                .map(|(step, status)| {
                    let (icon, description) = match status {
                        StepStatus::Complete => (
                            Icon::Check
                                .render(14., self.palette.success)
                                .into_any_element(),
                            "accounts.auth.verification.step.complete",
                        ),
                        StepStatus::Active => (
                            WaitRing::element(self.palette, step.label(), animate),
                            "accounts.auth.verification.step.active",
                        ),
                        StepStatus::Pending => (
                            Icon::Info
                                .render(14., self.palette.muted)
                                .into_any_element(),
                            "accounts.auth.verification.step.pending",
                        ),
                    };
                    let label = tr!(language, step.label());
                    let row = div()
                        .id(step.label())
                        .aria_label(label.clone())
                        .aria_description(tr!(language, description))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon)
                        .child(note(label).min_w_0());
                    match status {
                        StepStatus::Active => row.role(Role::Status),
                        StepStatus::Complete | StepStatus::Pending => row.role(Role::ListItem),
                    }
                }),
        )
    }
}
