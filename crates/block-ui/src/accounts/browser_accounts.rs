//! Account inspection presents identity and authority without changing selection.

use block_auth::{Account, AccountProvider};
use gpui::{ParentElement as _, Styled as _, div, rgb};

use super::{Block, RowIcon, account_id, components::BadgeVariant, note};
use crate::authentication::provider_label;

impl Block {
    pub(super) fn account_details(&self, account: &Account, current: bool) -> gpui::Div {
        let language = self.language();
        let id = account_id(account);
        let (authentication, identity, credentials) = match account.provider {
            AccountProvider::Microsoft => (
                tr!(language, "accounts.details.authentication.microsoft"),
                tr!(language, "accounts.details.identity.microsoft"),
                tr!(language, "accounts.details.credentials.microsoft"),
            ),
            AccountProvider::Authlib => (
                tr!(language, "accounts.details.authentication.authlib"),
                tr!(language, "accounts.details.identity.authlib"),
                tr!(language, "accounts.details.credentials.authlib"),
            ),
            AccountProvider::Offline => (
                tr!(language, "accounts.details.authentication.offline"),
                tr!(language, "accounts.details.identity.offline"),
                tr!(language, "accounts.details.credentials.offline"),
            ),
        };
        let player_name = tr!(language, "accounts.details.player_name");
        let account_type = tr!(language, "accounts.details.account_type");
        let authentication_label = tr!(language, "accounts.details.authentication");
        let environment = tr!(language, "accounts.details.environment");
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.detail_identity(
                RowIcon::Account,
                &account.player_name,
                self.account_avatars.get(&id),
                vec![(self.account_authority(account), BadgeVariant::Info)],
                current,
            ))
            .child(Self::detail_metadata(
                &tr!(language, "accounts.details.title"),
                &[
                    (player_name.as_str(), account.player_name.clone()),
                    (
                        account_type.as_str(),
                        provider_label(language, account.provider),
                    ),
                    (authentication_label.as_str(), authentication),
                    (environment.as_str(), "Minecraft: Java Edition".into()),
                ],
            ))
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .pt_5()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(note(identity))
                    .child(note(credentials)),
            )
            .children(
                (account.provider == AccountProvider::Authlib)
                    .then(|| self.account_provider_details(account)),
            )
    }

    fn account_provider_details(&self, account: &Account) -> gpui::Div {
        let language = self.language();
        let name = tr!(language, "accounts.details.provider_name");
        let api = tr!(language, "accounts.details.provider_api");
        let linked_account = tr!(language, "accounts.details.provider_account");
        Self::detail_metadata(
            &tr!(language, "accounts.details.provider"),
            &[
                (name.as_str(), self.account_authority(account)),
                (
                    api.as_str(),
                    self.account_provider(account).map_or_else(
                        || tr!(language, "accounts.details.provider_unavailable"),
                        |provider| provider.api_url.clone(),
                    ),
                ),
                (
                    linked_account.as_str(),
                    account
                        .authlib
                        .as_ref()
                        .map_or_else(String::new, |link| link.login.clone()),
                ),
            ],
        )
    }
}
