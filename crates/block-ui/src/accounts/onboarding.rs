//! Shared first-run completion and account draft state.
#![expect(
    clippy::redundant_pub_crate,
    reason = "First-run state is shared only with the parent view."
)]

use super::{Block, account_id};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub account: Option<String>,
    pub account_decided: bool,
}

impl Block {
    pub(super) fn complete_onboarding(&mut self) -> bool {
        let mut preferences = self.preferences.clone();
        preferences.onboarding_complete = true;
        let Some(engine) = &self.engine else {
            self.report(
                "Setup could not be saved. Check Block's data directory and restart.".into(),
            );
            return false;
        };
        if let Err(error) = preferences.save(&engine.root) {
            self.report(format!("Could not finish setup: {error}"));
            return false;
        }
        if let Some(engine) = &self.engine {
            engine.discard_prepared_sign_in();
        }
        self.selected_account = self.shell.onboarding.account.clone().filter(|id| {
            self.accounts
                .iter()
                .any(|account| account_id(account) == *id)
        });
        self.saved_preferences = preferences.clone();
        self.preferences = preferences;
        self.router.navigate(super::Tab::Home);
        self.form = None;
        if self.shell.first_run.enabled {
            self.normalize_selection();
        }
        self.pending_window_size = None;
        self.shell.revision = self.shell.revision.wrapping_add(1);
        self.push_toast(
            "Setup saved · Continue on Home".into(),
            super::feedback::ToastKind::Success,
        );
        true
    }
}

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
