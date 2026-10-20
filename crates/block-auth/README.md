# Block authentication

This crate implements the public desktop Microsoft Authorization Code flow with
PKCE, a one-use IPv4 loopback callback, Xbox Live authentication, XSTS, Minecraft
Services login, Java entitlement verification and the Java player profile.
It does not accept a client secret or read environment files.

## Desktop integration

1. Configure `MicrosoftAuth::new` with the application's public client ID.
2. Call `begin_sign_in`. It first checks native secure storage using a fresh,
   synthetic credential, then binds `127.0.0.1` on an ephemeral port.
3. Pass `PendingSignIn::authorization_url()` directly to the system browser.
   Do not print the URL: it contains ephemeral OAuth state.
4. Call `complete_sign_in(pending, &cancellation)` on the worker runtime. The
   callback expires after ten minutes. The returned account exists only after
   Java ownership, profile and secure credential storage have succeeded.
5. Persist only `Account` metadata in the launcher's catalog. Store account
   references in Profiles, rather than refresh tokens. If catalog persistence
   fails after sign-in, remove only a newly introduced credential reference with
   `sign_out` and report the failure. When an existing account reauthorizes the
   same reference, retain its rotated credential so its saved metadata remains
   usable. Signing in the same player with a different application client ID
   requires signing out the saved account first.
6. Call `refresh(&account, &cancellation)` before a launch to obtain a fresh,
   verified session. Refresh rechecks ownership and player UUID. A rotated refresh
   token is stored before later Xbox/Minecraft requests so a network failure does
   not lose the rotation.
7. Convert the session's player identity and explicitly exposed access token into
   an in-memory launch command. Sessions cannot be serialized. Their Debug output
   redacts tokens. The launcher must redact the resulting argv and game logs too.
8. Delete credentials with `sign_out`; delete catalog metadata only after that
   operation succeeds. Signing out removes the app's local credentials and does
   not remotely revoke an already running game's session.

`Account` metadata is not evidence of a currently valid session. `game_minecraft`
is required in the HTTPS entitlements response; a Microsoft account, access token
or Minecraft profile alone never bypasses that check. Entitlement signatures are
not verified offline. No offline grace period is implemented by this crate.

## Application registration

Use a personal-account-capable **Mobile and desktop applications** registration
with `http://localhost/block/callback`. This dedicated path must belong only to
the desktop platform. Microsoft ignores the ephemeral localhost port but
preserves the path. Existing Web/SPA callbacks can remain registered at different
paths; adding a native callback with only a different port does not separate them.
An observed registration had `/callback` under all three platforms and rejected
the public desktop token exchange. Block now uses its own path to avoid that
ambiguity without removing existing callbacks. A successful confidential Web
flow using a secret does not validate public desktop registration.
The API routes are explicitly
`authentication/login_with_xbox` and `entitlements/mcstore`, without silent
fallback to another route.

## Secure storage

- macOS: Keychain (`apple-native`).
- Windows: Credential Manager (`windows-native`).
- Linux: encrypted D-Bus Secret Service (`sync-secret-service`, `crypto-rust`).

Credentials belong to service `dev.block.launcher.microsoft`, with references
containing only the public application client ID and player UUID. There is no
plaintext, mock-store, browser storage, or kernel-only persistence fallback.
Linux without an unlocked Secret Service fails explicitly. Unsupported operating
systems fail explicitly too. Only owned secret strings and collected response
bodies are zeroized on drop; the HTTP stack also necessarily holds transient
request/response buffers.

The explicit diagnostic below writes, reads and deletes a **new synthetic entry**;
it does not enumerate, read or copy existing account credentials. Native stores
may ask the user to unlock or approve access.

```sh
cargo run -p block-auth --example secure-store-probe
```

## Verification

```sh
cargo test -p block-auth
cargo clippy -p block-auth --all-targets -- -D warnings
```

Tests use synthetic responses and local loopback HTTP servers. They verify PKCE,
callback state matching, duplicate rejection, cancellation, identity consistency,
HTTP error sanitization, ownership rejection, profile parsing, session expiration
and metadata/Debug redaction. They do not prove a real application registration,
Microsoft consent, a real account's ownership or native-store availability.
Those require a new user-authorized desktop login on each test host; credentials
must never be copied between hosts.

## Primary references

- [Microsoft authorization code flow and PKCE](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)
- [Microsoft localhost redirect matching and platform restrictions](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url#localhost-exceptions)
- [Microsoft Xbox user and XSTS authentication](https://learn.microsoft.com/en-us/xbox/gdk/docs/services/fundamentals/s2s-auth-calls/service-authentication/live-website-authentication)
- [XMCL's Minecraft Services implementation](https://github.com/voxelum/minecraft-launcher-core-node/blob/master/packages/user/microsoft.ts)
- [Prism's Microsoft scope and public client configuration](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/steps/MSAStep.cpp)
- [keyring 3.6.3 platform stores](https://docs.rs/keyring/3.6.3/keyring/)
