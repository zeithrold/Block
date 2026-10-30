//! Account inspection presents identity and authority without changing selection.

use block_auth::{Account, AccountProvider};
use gpui::{ParentElement as _, Styled as _, div, rgb};

use super::{Block, RowIcon, account_id, components::BadgeVariant, note};

impl Block {
    pub(super) fn account_details(&self, account: &Account, current: bool) -> gpui::Div {
        let id = account_id(account);
        let (authentication, identity, credentials) = match account.provider {
            AccountProvider::Microsoft => (
                "Ownership rechecked at launch",
                "A Microsoft identity for Minecraft: Java Edition.",
                "Sign-in and ownership are checked again when launching. Credentials remain in your system’s secure credential store.",
            ),
            AccountProvider::Authlib => (
                "External authentication provider",
                "An identity issued by an external authentication provider.",
                "Provider credentials must be verified before launch. Access and client tokens belong in your system credential store.",
            ),
            AccountProvider::Offline => (
                "Local identity; no online authorization",
                "A local player identity for offline play.",
                "The UUID is derived from the exact player name. No credentials are stored. This identity cannot access authenticated online services.",
            ),
        };
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
                "Account details",
                &[
                    ("Player name", account.player_name.clone()),
                    ("Account type", account.provider.label().into()),
                    ("Authentication", authentication.into()),
                    ("Environment", "Minecraft: Java Edition".into()),
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
        Self::detail_metadata(
            "Provider",
            &[
                ("Name", self.account_authority(account)),
                (
                    "API address",
                    self.account_provider(account).map_or_else(
                        || "Provider unavailable".into(),
                        |provider| provider.api_url.clone(),
                    ),
                ),
                (
                    "Account",
                    account
                        .authlib
                        .as_ref()
                        .map_or_else(String::new, |link| link.login.clone()),
                ),
            ],
        )
    }
}
