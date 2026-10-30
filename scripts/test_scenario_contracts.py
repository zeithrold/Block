from __future__ import annotations

import unittest

from scenario_contracts import (
    HOME_VIEWPORT,
    MANAGEMENT_VIEWPORT,
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
                self.assertIn(value["viewport"], {HOME_VIEWPORT, MANAGEMENT_VIEWPORT})
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
            self.assertEqual(value["viewport"], MANAGEMENT_VIEWPORT)
            self.assertIn(phrase, value["required"])
            observed[name] = value["required"]
        self.assertEqual(len({tuple(value) for value in observed.values()}), 4)

    def test_unknown_or_uncontracted_scenarios_fail_closed(self) -> None:
        with self.assertRaises(KeyError):
            contract("home-does-not-exist")
        names = scenario_names()
        for name in (
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
                self.assertEqual(contract(name)["viewport"], MANAGEMENT_VIEWPORT)


if __name__ == "__main__":
    unittest.main()
