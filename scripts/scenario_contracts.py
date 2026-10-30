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
MANAGEMENT_VIEWPORT = (1152, 720)
_WELCOME_FRAME_SCENARIOS = {"setup-welcome", "setup-import-sources", "setup-import-list"}


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
_add("games-detail", "Vanilla", "Minecraft version")
_add("games-instance-settings", "Instance configuration", "Instance name")
_add("remove-confirmation", "Delete Vanilla?", "This game and its launch settings are removed from the library.")
_add("remove-account-confirmation", "Remove Zeithrold?", "Your games and worlds stay on disk.")
_add("separate-confirmation", "Enable separated profiles?", "Bundled mode cannot be restored.")
_add("games-help", "An Instance fixes the game version and mod loader.", "A Profile stores your launch preferences.")

_add("wizard-setup", "Create a game", "Game name")
_add("wizard-review", "Create a game", "Create game saves your setup and begins verified downloads.")
_add("wizard-optifine", "Create a game", "OptiFine")
_add("wizard-optifine-selected", "Create a game", "OptiFine")
_add("wizard-version-picker", "Create a game", "Search versions")
_add("wizard-loader-picker", "Create a game", "Search loader builds")
_add("wizard-fabric-picker", "Create a game", "Fabric versions")
_add("wizard-quilt-picker", "Create a game", "Quilt versions")
_add("wizard-neoforge-picker", "Create a game", "NeoForge versions")
_add("wizard-forge-picker", "Create a game", "Forge versions")
_add("wizard-loader-loading", "Fabric versions", "Loading official builds…")
_add("wizard-loader-error", "Fabric versions", "The official catalog could not be reached.")
_add("wizard-loader-empty", "Fabric versions", "No compatible builds")
_add("wizard-loader-search-empty", "Fabric versions", "No matching builds")
_add("wizard-configuration", "Create a game", "Installation folder (optional)")

_add("accounts-list", "Your accounts", "Add account")
_add("accounts-detail", "Your accounts", "Zeithrold")
_add("accounts-offline-detail", "Your accounts", "Pixel")
_add("accounts-providers", "Authlib providers", "Add a provider")
_add("provider-add", "Add a provider", "Drop an Authlib quick link here")
_add("provider-http-warning", "This provider uses HTTP.", "Use HTTPS whenever possible.")
_add("remove-provider-confirmation", "Remove this provider?", "The saved provider and cached metadata are removed.")

_add("auth-choose", "Account setup", "Choose how to sign in")
_add("auth-information", "Account setup", "Sign in with Microsoft")
_add("auth-authlib-information", "Account setup", "Sign in to your provider")
_add("auth-authlib-http-warning", "Account setup", "This provider uses HTTP.")
_add("auth-authlib-characters", "Choose your character", "Explorer")
_add("auth-authlib-complete", "Your account is ready", "Zeithrold")
_add("auth-authlib-provider-add", "Add a provider", "Drop an Authlib quick link here")
_add("auth-offline-information", "Your local player", "Choose a name for local play")
_add("auth-offline-error", "Check your player name", "Use 1–16 ASCII letters, numbers, or underscores.")
_add("auth-offline-complete", "Your account is ready", "Pixel")
_add("auth-device-preparing", "Preparing secure sign-in", "Block is requesting a real Microsoft authorization session.")
_add("auth-device-waiting", "Approve your device code", "TEST-CODE")
_add("auth-waiting", "Continue in your browser", "Open browser again")
_add("auth-verifying", "Verifying your account", "Block is checking Java Edition ownership")
_add("auth-complete", "Your account is ready", "Zeithrold")
_add("auth-saving", "Saving your account", "Block is saving the refresh credential")
_add("auth-error", "Sign-in needs your attention", "The authorization request expired.")

_add("settings-general", "General", "Appearance")
_add("settings-game", "Game defaults", "Defaults apply to new Instances and Profiles.")
_add("settings-profiles", "Profile mode", "Separated Profile")
_add("settings-downloads", "Downloads & network", "Parallel downloads")
_add("settings-socks5", "Downloads & network", "socks5h://127.0.0.1:1080")
_add("settings-storage", "Storage", "Game storage", "Download cache")
_add("about", "MINECRAFT: JAVA EDITION LAUNCHER", "Made for your next world.")
_add("setup-welcome", "Welcome Back", "Add an account and set up a game.", "Skip setup")
_add("setup-overview", "Two steps and you're ready", "Account", "Game")
_add("setup-overview-ready", "Two steps and you're ready", "Finish")
_add("setup-game-choice", "Set up a game", "Recommended setup", "Custom setup")
_add("setup-account", "Account", "Microsoft")
_add("setup-game", "Create a game", "Minecraft")
_add("setup-import-sources", "Import games", "Choose a source.")
_add("setup-import-list", "Minecraft Launcher", "Select games to copy.")


def _window_title(name: str) -> str:
    if name.startswith("auth-"):
        page = "Accounts"
    elif name in {"accounts-providers", "provider-add", "provider-http-warning", "remove-provider-confirmation"}:
        page = "Authlib providers"
    elif name == "games-instance-settings":
        page = "Instance settings"
    elif name.startswith("wizard-"):
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
    home = (
        name.startswith("home-")
        or name.startswith("toast-")
        or name == "download-files"
        or name in _WELCOME_FRAME_SCENARIOS
    )
    return {
        "title": _window_title(name),
        "viewport": HOME_VIEWPORT if home else MANAGEMENT_VIEWPORT,
        "required": list(required),
    }
