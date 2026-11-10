pub(crate) fn is_excluded_entry(name: &str, is_directory: bool, top_level: bool) -> bool {
    let normalized = name.to_ascii_lowercase();
    let launcher_profile =
        normalized.starts_with("launcher_profiles") || normalized.starts_with("launcher_accounts");
    let credential_file = !is_directory
        && matches!(
            normalized.as_str(),
            "account.json"
                | "accounts.json"
                | "accounts.dat"
                | "usercache.json"
                | "launcher_msa_credentials.bin"
                | "launcher_authlib_injector.json"
                | "credentials.json"
                | "credential.json"
                | "auth.json"
                | "secret.json"
                | "secrets.json"
                | "token.json"
                | "tokens.json"
                | "token"
                | "tokens"
                | "session.json"
                | "session.lock"
                | "session"
                | ".block-game.lock"
        );
    let environment_file = normalized == ".env" || normalized.starts_with(".env.");
    let sensitive_directory = is_directory
        && matches!(
            normalized.as_str(),
            ".git"
                | ".ssh"
                | ".aws"
                | ".gnupg"
                | ".kube"
                | ".azure"
                | ".docker"
                | ".config"
                | ".cache"
                | ".local"
        );
    let plain_sensitive_directory = top_level
        && is_directory
        && matches!(
            normalized.as_str(),
            "credentials" | "accounts" | "auth" | "sessions" | "tokens" | "secrets"
        );
    launcher_profile
        || credential_file
        || environment_file
        || sensitive_directory
        || plain_sensitive_directory
}
