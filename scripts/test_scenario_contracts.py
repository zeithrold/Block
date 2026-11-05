from __future__ import annotations

import unittest

from scenario_contracts import (
    HOME_VIEWPORT,
    CAPTURE_CONTRACT_REVISION,
    _REQUIRED,
    contract,
    scenario_names,
)


class ScenarioContractsTest(unittest.TestCase):
    def test_every_native_scenario_has_an_explicit_contract(self) -> None:
        names = scenario_names()
        self.assertEqual(names, set(_REQUIRED))
        for name in names:
            with self.subTest(name=name):
                value = contract(name)
                self.assertTrue(value["title"])
                self.assertEqual(value["viewport"], (360, 720))
                self.assertEqual(value["contract_revision"], CAPTURE_CONTRACT_REVISION)
                self.assertGreaterEqual(len(value["required"]), 2)
                self.assertEqual(len(value["required"]), len(set(value["required"])))

    def test_home_error_and_loader_states_have_distinct_visible_contracts(self) -> None:
        error = contract("home-error")
        self.assertEqual(error["title"], "Block")
        self.assertEqual(error["viewport"], HOME_VIEWPORT)
        self.assertIn("Retry download", error["required"])

        expected = {
            "wizard-loader-search-empty": "No matching builds",
            "wizard-loader-empty": "No compatible builds",
            "wizard-loader-loading": "Loading official builds…",
            "wizard-loader-error": "The official catalog could not be reached.",
        }
        observed = {}
        for name, phrase in expected.items():
            value = contract(name)
            self.assertEqual(value["title"], "Block · Create a game")
            self.assertEqual(value["viewport"], (360, 720))
            self.assertIn(phrase, value["required"])
            observed[name] = value["required"]
        self.assertEqual(len({tuple(value) for value in observed.values()}), 4)

    def test_unknown_or_uncontracted_scenarios_fail_closed(self) -> None:
        with self.assertRaises(KeyError):
            contract("home-does-not-exist")
        names = scenario_names()
        for name in (
            "wizard-review",
            "auth-offline-complete",
            "welcome-1",
            "welcome-2",
            "welcome-3",
            "welcome-version-picker",
            "welcome-settings",
            "welcome-about",
        ):
            with self.subTest(name=name):
                self.assertNotIn(name, names)
                with self.assertRaises(KeyError):
                    contract(name)

    def test_submission_boundaries_remain_distinguishable(self) -> None:
        self.assertIn("Game name", contract("wizard-setup")["required"])
        self.assertIn("Create and prepare", contract("wizard-setup")["required"])
        self.assertIn(
            "Creates the game and starts preparation. Minecraft and loader become fixed.",
            contract("wizard-setup")["required"],
        )
        self.assertIn("Apply to draft", contract("wizard-configuration")["required"])
        self.assertIn("Add and use", contract("auth-offline-information")["required"])
        self.assertIn(
            "Block is saving the local identity.",
            contract("auth-offline-saving")["required"],
        )
        for name in ("auth-complete", "auth-authlib-complete"):
            with self.subTest(name=name):
                self.assertIn("Save and use", contract(name)["required"])
        self.assertIn("Go to Home", contract("setup-overview")["required"])

    def test_first_run_scenarios_follow_their_page_titles_and_frames(self) -> None:
        for name in ("setup-welcome", "setup-import-sources", "setup-import-list"):
            with self.subTest(name=name):
                self.assertEqual(contract(name)["title"], "Block · Set up Block")
                self.assertEqual(contract(name)["viewport"], HOME_VIEWPORT)
        for name, title in (
            ("setup-overview", "Set up Block"),
            ("setup-overview-ready", "Set up Block"),
            ("setup-game-choice", "Set up a game"),
            ("setup-account", "Accounts"),
            ("setup-game", "Create a game"),
        ):
            with self.subTest(name=name):
                self.assertEqual(contract(name)["title"], f"Block · {title}")
                self.assertEqual(contract(name)["viewport"], (360, 720))

    def test_compact_details_do_not_require_the_unmounted_list(self) -> None:
        for name in ("accounts-detail", "accounts-offline-detail"):
            with self.subTest(name=name):
                self.assertIn("Account details", contract(name)["required"])
                self.assertNotIn("Your accounts", contract(name)["required"])
        vanilla = contract("wizard-loader-picker")["required"]
        self.assertIn("No mod loader needed", vanilla)
        self.assertNotIn("Search loader builds", vanilla)
        self.assertIn("Your providers", contract("accounts-providers")["required"])
        self.assertIn("Display name", contract("provider-add")["required"])

    def test_editor_dialogs_and_crash_use_their_actual_native_titles(self) -> None:
        for name, title in (
            ("instance-editor-exit-dirty", "Instance settings"),
            ("profile-editor-exit-dirty", "Launch settings"),
            ("provider-editor-exit-dirty", "Authlib providers"),
        ):
            with self.subTest(name=name):
                value = contract(name)
                self.assertEqual(value["title"], f"Block · {title}")
                self.assertEqual(value["required"], [
                    "Save your changes?", "Save and leave", "Discard changes", "Keep editing",
                ])
        for name in ("crash-overview", "crash-logs", "crash-long-metadata"):
            with self.subTest(name=name):
                value = contract(name)
                self.assertEqual(value["title"], "Block · Crash help")
                self.assertIn("Return to instance", value["required"])
        self.assertIn("Find in current text", contract("crash-logs")["required"])

    def test_settings_scenarios_use_the_home_sized_frame(self) -> None:
        settings = {name for name in scenario_names() if name.startswith("settings-")}
        self.assertTrue(settings)
        for name in settings:
            with self.subTest(name=name):
                self.assertEqual(contract(name)["title"], "Block · Settings")
                self.assertEqual(contract(name)["viewport"], HOME_VIEWPORT)

    def test_settings_index_exit_and_validation_have_distinct_contracts(self) -> None:
        index = contract("settings-index")
        self.assertIn("Choose what to adjust", index["required"])
        self.assertIn("Downloads & network", index["required"])

        exit_dialog = contract("settings-exit-dirty")
        for phrase in (
            "Return to Home?",
            "Save and leave",
            "Discard changes",
            "Keep editing",
        ):
            self.assertIn(phrase, exit_dialog["required"])

        invalid = contract("settings-invalid-proxy")
        self.assertIn("Changes were not saved.", invalid["required"])
        self.assertIn("Proxy settings are invalid", invalid["required"])
        self.assertNotIn("Choose what to adjust", invalid["required"])


if __name__ == "__main__":
    unittest.main()
