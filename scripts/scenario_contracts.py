#!/usr/bin/env python3
"""Source-backed visual assertions for native UI scenarios.

A scenario stem is only accepted when it has an explicit contract here. Titles,
viewports, and required OCR phrases are derived from the debug scenario setup,
window geometry, and visible GPUI labels; a filename alone proves nothing.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCENARIO_SOURCE = ROOT / "crates/block-ui/src/runtime/scenarios.rs"
HOME_VIEWPORT = (360, 720)
CAPTURE_CONTRACT_REVISION = "compact-ui-20261105"


def scenario_names(source: Path = SCENARIO_SOURCE) -> set[str]:
    text = source.read_text()
    match = re.search(r"pub const SCENARIOS: &\[&str\] = &\[(.*?)\n\];", text, re.S)
    if not match:
        raise ValueError(f"could not read SCENARIOS from {source}")
    return set(re.findall(r'"([a-z0-9-]+)"', match.group(1)))


# Each entry names visible copy from the relevant page or overlay. Keep phrases
# short enough for OCR while still distinguishing the state under review.
_REQUIRED: dict[str, tuple[str, ...]] = {}
_GAMES_SCENARIOS = {
    "games-bundled",
    "games-separated",
    "games-detail",
    "games-instance-settings",
    "remove-confirmation",
    "separate-confirmation",
    "games-help",
    "games-missing-record",
    "games-detail-long-path",
}


def _add(names: str, *required: str) -> None:
    for name in names.split():
        _REQUIRED[name] = tuple(required)


_add("home-empty", "Welcome to Block", "Add an account")
_add("home-ready", "Zeithrold", "Launch")
_add("home-downloading", "Downloading Minecraft resources", "Zeithrold")
_add(
    "home-error",
    "The download was interrupted. Check your connection and retry.",
    "Retry download",
)
_add("home-profile-drawer", "Switch Profile", "Choose how you want to play.")
_add("home-account-drawer", "Switch Account", "Choose the account you want to use.")
_add("home-downloads-drawer", "Downloads", "Track your download tasks.")
_add("toast-motion toast-motion-reduced", "Zeithrold", "Launch")
_add("download-files", "Observed resources", "Resource IDs are SHA-1 hashes")

_add("games-bundled", "Your games", "Vanilla")
_add("games-separated", "Instances", "Profiles")
_add("games-detail games-detail-long-path", "Vanilla", "Minecraft", "Edit launch settings")
_add("games-missing-record", "Item no longer available", "Go back to the list to choose another.")
_add("games-instance-settings", "Instance configuration", "Instance name")
_add("profile-editor-root", "Launch settings", "Server address (optional)", "Save changes")
_add("profile-editor-data", "Data and account", "Default launch account", "Apply to draft")
_add("profile-editor-launch", "Launch settings", "Maximum memory (GB)", "Apply to draft")
_add("profile-editor-appearance", "Profile appearance", "Apply to draft")
_add(
    "instance-editor-exit-dirty profile-editor-exit-dirty provider-editor-exit-dirty",
    "Save your changes?", "Save and leave", "Discard changes", "Keep editing",
)
_add("remove-confirmation", "Delete Vanilla?", "This game and its launch settings are removed from the library.")
_add("remove-account-confirmation", "Remove Zeithrold?", "Your games and worlds stay on disk.")
_add("separate-confirmation", "Enable separated profiles?", "Bundled mode cannot be restored.")
_add("games-help", "An Instance fixes the game version and mod loader.", "A Profile stores your launch preferences.")

_add(
    "wizard-setup",
    "Create your game",
    "Game name",
    "Create and prepare",
    "Creates the game and starts preparation. Minecraft and loader become fixed.",
)
_add("wizard-optifine", "Create a game", "OptiFine")
_add("wizard-optifine-selected", "Create a game", "OptiFine")
_add("wizard-version-picker", "Create a game", "Search versions")
_add("wizard-loader-picker", "Mod loader", "Vanilla", "No mod loader needed")
_add("wizard-fabric-picker", "Create a game", "Fabric versions")
_add("wizard-quilt-picker", "Create a game", "Quilt versions")
_add("wizard-neoforge-picker", "Create a game", "NeoForge versions")
_add("wizard-forge-picker", "Create a game", "Forge versions")
_add("wizard-loader-loading", "Fabric versions", "Loading official builds…")
_add("wizard-loader-error", "Fabric versions", "The official catalog could not be reached.")
_add("wizard-loader-empty", "Fabric versions", "No compatible builds")
_add("wizard-loader-search-empty", "Fabric versions", "No matching builds")
_add("wizard-configuration", "Configuration", "Installation folder (optional)", "Apply to draft")
_add("wizard-configuration-invalid", "Configuration", "Choose an absolute installation folder or leave it empty.", "Apply to draft")

_add("accounts-list", "Your accounts", "Add account")
_add("accounts-detail", "Account details", "Zeithrold", "Current account")
_add("accounts-offline-detail", "Account details", "Pixel", "Use account")
_add("accounts-providers", "Your providers", "Community server", "Local community", "Add a provider")
_add("provider-add", "Add a provider", "Display name", "Server API address or quick link")
_add("provider-detail", "Community server", "Edit provider", "Fixture Community")
_add("provider-edit", "Edit provider", "Display name", "Community server", "Save changes")
_add("provider-http-warning", "This provider uses HTTP.", "Use HTTPS whenever possible.")
_add("remove-provider-confirmation", "Remove this provider?", "The saved provider and cached metadata are removed.")

_add("auth-choose", "Sign in with Microsoft", "Hide other sign-in options")
_add("auth-information", "Sign in with Microsoft", "Open browser")
_add("auth-authlib-information", "Sign in to your provider", "Sign in")
_add("auth-authlib-http-warning", "Sign in to your provider", "This provider uses HTTP.")
_add("auth-authlib-characters", "Choose your character", "Explorer")
_add("auth-authlib-complete", "Your account is ready", "Zeithrold", "Save and use")
_add("auth-authlib-provider-add", "Add a provider", "Drop an Authlib quick link here")
_add("auth-offline-information", "Your local player", "Add and use")
_add("auth-offline-error", "Check your player name", "Use 1–16 ASCII letters, numbers, or underscores.")
_add("auth-offline-saving", "Saving your account", "Block is saving the local identity.")
_add("auth-device-preparing", "Preparing secure sign-in", "Block is requesting a real Microsoft authorization session.")
_add("auth-device-waiting", "Approve your device code", "TEST-CODE")
_add("auth-waiting", "Continue in your browser", "Open browser again")
_add("auth-verifying", "Verifying your account", "Block is checking Java Edition ownership")
_add("auth-complete", "Your account is ready", "Zeithrold", "Save and use")
_add("auth-saving", "Saving your account", "Block is saving the refresh credential")
_add("auth-error", "Sign-in needs your attention", "Authorization expired. Start sign-in again to request a fresh session.")

_add("settings-index", "Choose what to adjust", "Game defaults", "Downloads & network")
_add(
    "settings-exit-dirty",
    "Return to Home?",
    "Save and leave",
    "Discard changes",
    "Keep editing",
)
_add(
    "settings-invalid-proxy",
    "Downloads & network",
    "Changes were not saved.",
    "Proxy settings are invalid",
)
_add("settings-general", "General", "Appearance")
_add("settings-game", "Game defaults", "Defaults apply to new Instances and Profiles.")
_add("settings-profiles", "Profile mode", "Separated Profile")
_add("settings-downloads", "Downloads & network", "Parallel downloads")
_add("settings-socks5", "Downloads & network", "socks5h://127.0.0.1:1080")
_add("settings-storage", "Storage", "Game storage", "Download cache")
_add("about", "One block at a time.", "Build information")
_add("setup-welcome", "Welcome Back", "Add an account and set up a game.", "Skip setup")
_add("setup-overview", "Set up Block", "Account", "Game", "Go to Home")
_add("setup-overview-ready", "Set up Block", "Go to Home")
_add("setup-overview-draft", "Set up Block", "A game draft is still open.", "Continue draft")
_add("setup-game-choice", "Set up a game", "Recommended setup", "Custom setup")
_add("setup-account", "Sign in with Microsoft", "Open browser")
_add("setup-game", "Create your game", "Minecraft")
_add("setup-import-sources", "Import games", "Choose a source.")
_add("setup-import-list", "Minecraft Launcher", "Select games to copy.")
_add("setup-import-long-destination", "Minecraft Launcher", "Select games to copy.", "a deliberately long destination folder")

_add("modpack-choose", "Import a modpack", "Choose archive")
_add("modpack-inspecting", "Import a modpack", "Reading the archive")
_add("modpack-review", "Import a modpack", "Compact workshop", "Review what the pack installs before writing it into a new instance.")
_add("modpack-blocked", "Import a modpack", "This modpack cannot be imported as inspected.")
_add("modpack-stopped", "Import stopped", "The installation is not finished.")
_add("modpack-installing", "Importing the modpack", "Installing the pack's files", "Stop")
_add("modpack-complete", "Modpack imported", "Compact workshop", "Done")
_add("crash-overview", "Minecraft closed unexpectedly", "Java heap memory may have been exhausted", "Return to instance")
_add("crash-logs", "Minecraft closed unexpectedly", "Recent game output", "Find in current text", "Return to instance")
_add("crash-long-metadata", "Minecraft closed unexpectedly", "Creative worlds and archived experiments with a deliberately long Profile name", "Return to instance")


def _window_title(name: str) -> str:
    if name.startswith("crash-"):
        page = "Crash help"
    elif name.startswith("auth-"):
        page = "Accounts"
    elif name.startswith("provider-") or name in {"accounts-providers", "remove-provider-confirmation"}:
        page = "Authlib providers"
    elif name in {"games-instance-settings", "instance-editor-exit-dirty"}:
        page = "Instance settings"
    elif name == "profile-editor-data":
        page = "Data and account"
    elif name == "profile-editor-appearance":
        page = "Profile appearance"
    elif name.startswith("profile-editor-"):
        page = "Launch settings"
    elif name.startswith("wizard-") or name.startswith("modpack-"):
        page = "Create a game"
    elif name.startswith("settings-"):
        page = "Settings"
    elif name == "about":
        page = "About Block"
    elif name == "setup-account":
        page = "Accounts"
    elif name == "setup-game":
        page = "Create a game"
    elif name == "setup-game-choice":
        page = "Set up a game"
    elif name.startswith("setup-"):
        page = "Set up Block"
    elif name.startswith("accounts-") or name == "remove-account-confirmation":
        page = "Accounts"
    elif name.startswith("home-") or name.startswith("toast-") or name == "download-files":
        return "Block"
    elif name in _GAMES_SCENARIOS:
        page = "Games"
    else:
        raise KeyError(f"native scenario has no window title contract: {name}")
    return f"Block · {page}"


def contract(name: str) -> dict[str, object]:
    """Return the explicit native capture contract for ``name``.

    Unknown or newly added scenario names fail closed until their native title
    and visible content have been reviewed and added here.
    """
    if name not in scenario_names():
        raise KeyError(f"unknown native scenario: {name}")
    required = _REQUIRED.get(name)
    if required is None:
        raise KeyError(f"native scenario has no visual contract: {name}")
    return {
        "title": _window_title(name),
        "viewport": HOME_VIEWPORT,
        "required": list(required),
        "contract_revision": CAPTURE_CONTRACT_REVISION,
    }
