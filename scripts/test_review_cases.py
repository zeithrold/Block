#!/usr/bin/env python3
"""Focused regression tests for semantic pairing and evidence reuse."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import review_cases
from scenario_contracts import contract

# Scenario stems that existed in archived captures but are no longer produced by
# crates/block-ui/src/runtime/scenarios.rs. Their registry rows remain historical
# reference observations; they must not satisfy current-scenario coverage.
RETIRED_NATIVE_STEMS = {
    "auth-offline-complete",
    "wizard-review",
    "welcome-1",
    "welcome-2",
    "welcome-3",
    "welcome-version-picker",
    "welcome-settings",
    "welcome-about",
}


class ReviewCaseTests(unittest.TestCase):
    @staticmethod
    def _combined_case(platform, native_path, digest, receipt=None, reference_path="proto/shared-light.png"):
        case = {
            "case_id": "case-a",
            "review_status": "source_audited_difference",
            "object": {"type": "page", "identity": "case"},
            "trigger": {"value": "open"},
            "data": {"value": "seed"},
            "viewport": {"native": "registry unknown"},
            "focus": "registry unknown",
            "scroll": "registry unknown",
            "source_provenance": {"native_source": "registry only"},
            "comparable_scope": "visual_component_only",
            "rationale": "fixture",
            "artifacts": {
                "native": [{
                    "path": native_path,
                    "sha256": digest,
                    "theme": "light",
                    "size_px": {"width": 10, "height": 10},
                    "capture_receipt": receipt,
                }],
                "reference_candidates": [{
                    "path": reference_path,
                    "sha256": digest,
                    "theme": "light",
                    "size_px": {"width": 10, "height": 10},
                    "capture_receipt": None,
                }],
            },
        }
        return {"cases": [case], "reuse_groups": []}

    def test_known_same_name_mismatches_are_not_full_state_matches(self):
        registry = review_cases.load_registry()
        cases = {case["case_id"]: case for case in registry["cases"]}
        for stem in ("remove-confirmation", "provider-add", "wizard-loader-picker", "wizard-loader-empty", "auth-error"):
            self.assertNotEqual(cases[stem]["review_status"], "semantic_equivalent", stem)
            self.assertNotEqual(cases[stem]["comparable_scope"], "full_state", stem)

    def test_source_audit_records_actual_loader_and_provider_states(self):
        registry = review_cases.load_registry()
        cases = {case["case_id"]: case for case in registry["cases"]}
        historical = [case for case in cases.values() if case["lifecycle"] != "current"]
        self.assertTrue(historical)
        self.assertFalse(any(case["review_status"] == "unreviewed" for case in historical))

        vanilla = cases["wizard-loader-picker"]
        self.assertIn("Vanilla", vanilla["data"]["value"])
        self.assertIn("Fabric selected", vanilla["reference_audit"]["archive_visual_observation"])
        self.assertNotEqual(vanilla["review_status"], "semantic_equivalent")

        no_builds = cases["wizard-loader-empty"]
        search_empty = cases["wizard-loader-search-empty"]
        self.assertIn("zero builds", no_builds["data"]["value"])
        self.assertIn("query zzzz", no_builds["reference_audit"]["data"])
        self.assertEqual(search_empty["reference_candidates"], ["wizard-loader-empty"])
        self.assertFalse(search_empty["coverage_counts_as_independent"])

        provider = cases["provider-add"]["reference_audit"]
        self.assertEqual(provider["current_snap_proto_status"], "scripted_capture_but-existing-provider-edit")
        self.assertIn("initialProviders[0]", provider["trigger"])

    def test_current_native_stems_cover_scenarios_without_reusing_historical_reviews(self):
        cases = review_cases.load_registry()["cases"]
        retired_case_ids = {
            case["case_id"]
            for case in cases
            if case["lifecycle"] == "retired"
        }
        retired_stems = {
            stem for case in cases if case["case_id"] in retired_case_ids for stem in case["native_stems"]
        }
        self.assertEqual(retired_stems, RETIRED_NATIVE_STEMS)
        self.assertTrue(retired_stems.isdisjoint(review_cases.scenario_names()))
        for case in cases:
            if case["case_id"] in retired_case_ids:
                self.assertTrue(case["rationale"].startswith("Historical (retired scenario):"), case["case_id"])

        historical = [case for case in cases if case["lifecycle"] == "historical"]
        self.assertTrue(historical)
        self.assertTrue(all(case["review_status"] != "unreviewed" for case in historical))
        active = [case for case in cases if case["lifecycle"] == "current"]
        for case in active:
            self.assertEqual(case["review_status"], "unreviewed", case["case_id"])
            self.assertEqual(case["comparable_scope"], "not_assessed", case["case_id"])
            self.assertFalse(case["coverage_counts_as_independent"], case["case_id"])
            self.assertEqual(case["reference_candidates"], [], case["case_id"])
            for stem in case["native_stems"]:
                self.assertEqual(
                    case["capture_contract_revision"],
                    contract(stem)["contract_revision"],
                    case["case_id"],
                )
        active_stems = {stem for case in active for stem in case["native_stems"]}
        self.assertEqual(len(active_stems), sum(len(case["native_stems"]) for case in active))
        self.assertEqual(active_stems, review_cases.scenario_names())

        # The current first-run states are native-only until a React candidate is
        # actually reviewed; an empty mapping must not invent correspondence.
        setup_cases = [case for case in active if any(stem.startswith("setup-") for stem in case["native_stems"])]
        self.assertEqual(
            {stem for case in setup_cases for stem in case["native_stems"]},
            {name for name in review_cases.scenario_names() if name.startswith("setup-")},
        )
        for case in setup_cases:
            self.assertEqual(case["reference_candidates"], [], case["case_id"])

    def test_exact_filename_candidate_stays_unreviewed_and_duplicate_bytes_do_not_count(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native = root / "windows-amd64/scenarios"
            proto = root / "proto"
            native.mkdir(parents=True)
            proto.mkdir(parents=True)
            # Valid 1x1 PNG. The same bytes under matching filenames are still
            # just reused evidence, not independent semantic coverage.
            png = bytes.fromhex(
                "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
                "0000000b49444154789c636000020000050001a5f645400000000049454e44ae426082"
            )
            (native / "same-light.png").write_bytes(png)
            (proto / "same-light.png").write_bytes(png)
            case = {
                "case_id": "same",
                "native_stems": ["same"],
                "reference_candidates": ["same"],
                "review_status": "unreviewed",
                "object": {"type": "unknown", "identity": "unknown"},
                "trigger": {"value": "unknown"},
                "data": {"value": "unknown"},
                "viewport": {"native": "unknown"},
                "focus": "unknown",
                "scroll": "unknown",
                "source_provenance": {},
                "comparable_scope": "not_assessed",
                "rationale": "unreviewed",
            }
            registry_path = root / "registry.json"
            registry_path.write_text(json.dumps({"schema_version": 1, "cases": [case]}))
            with patch.object(review_cases, "OUTPUT", root), patch.object(review_cases, "ROOT", root):
                # The fixture registry is intentionally not the native source
                # registry, so test collection directly after validation.
                registry = review_cases.load_registry(registry_path)
                with patch.object(review_cases, "scenario_names", return_value={"same"}):
                    manifest = review_cases.build_semantic_manifest("windows-amd64", registry_path)
            result = manifest["cases"][0]
            self.assertEqual(result["review_status"], "unreviewed")
            self.assertFalse(result["coverage_counts_as_independent"])
            self.assertEqual(len(manifest["reuse_groups"]), 1)
            self.assertEqual(manifest["summary"]["reviewed_semantic_equivalences"], 0)

    def test_compact_receipt_routes_same_named_image_without_inheriting_archive_review(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native = root / "linux-arm64/scenarios"
            native.mkdir(parents=True)
            image = native / "same-light.png"
            image.write_bytes(bytes.fromhex(
                "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
                "0000000b49444154789c636000020000050001a5f645400000000049454e44ae426082"
            ))
            common = {
                "native_stems": ["same"], "reference_candidates": [],
                "object": {"type": "page", "identity": "same"},
                "trigger": {"value": "open"}, "data": {"value": "seed"},
                "viewport": {"native": "unknown"}, "focus": "unknown",
                "scroll": "unknown", "source_provenance": {}, "rationale": "fixture",
            }
            registry_path = root / "registry.json"
            registry_path.write_text(json.dumps({"schema_version": 1, "cases": [
                {**common, "case_id": "archive", "lifecycle": "historical",
                 "review_status": "semantic_equivalent", "comparable_scope": "full_state"},
                {**common, "case_id": "compact", "lifecycle": "current",
                 "capture_contract_revision": "compact-test",
                 "review_status": "unreviewed", "comparable_scope": "not_assessed"},
            ]}))
            for revision, passed, expected in (
                (None, True, (1, 0)),
                ("older-contract", True, (0, 0)),
                ("compact-test", False, (0, 0)),
                ("compact-test", True, (0, 1)),
            ):
                with self.subTest(revision=revision, passed=passed):
                    image.with_suffix(".capture.json").write_text(json.dumps({
                        "schema_version": 1,
                        "sha256": review_cases.sha256(image),
                        "assertions": {"contract_revision": revision, "passed": passed},
                    }))
                    with (
                        patch.object(review_cases, "OUTPUT", root),
                        patch.object(review_cases, "ROOT", root),
                        patch.object(review_cases, "scenario_names", return_value={"same"}),
                    ):
                        manifest = review_cases.build_semantic_manifest("linux-arm64", registry_path)
                    self.assertEqual(
                        tuple(len(case["artifacts"]["native"]) for case in manifest["cases"]),
                        expected,
                    )
                    self.assertEqual(manifest["summary"]["reviewed_semantic_equivalences"], 0)

    def test_combined_reuse_finds_cross_platform_duplicates_and_dedupes_shared_reference_path(self):
        digest = "a" * 64
        manifests = {
            "windows-amd64": self._combined_case("windows-amd64", "windows/scenarios/case-a-light.png", digest),
            "macos-arm64": self._combined_case("macos-arm64", "native/case-a-light.png", digest),
        }
        combined = review_cases.combine_platform_manifests(manifests)
        self.assertEqual(combined["summary"]["reviewed_semantic_equivalences"], 0)
        self.assertEqual(len(combined["reuse_groups"]), 1)
        # Two platform-native artifacts plus one shared reference artifact.
        self.assertEqual(len(combined["reuse_groups"][0]["artifacts"]), 3)
        paths = {artifact["path"] for artifact in combined["reuse_groups"][0]["artifacts"]}
        self.assertEqual(paths, {
            "windows/scenarios/case-a-light.png", "native/case-a-light.png", "proto/shared-light.png"
        })

    def test_cross_platform_markdown_reports_receipt_observations_without_hydrating_registry_fields(self):
        receipt = {
            "viewport": {"width": 1152, "height": 720, "dpr": 2},
            "focus": "Accounts search input",
            "scroll": "top",
            "source": {"commit": "abc123", "tracked_source_diff_sha256": "diff-hash"},
            "binary_sha256": "binary-hash",
        }
        manifests = {
            "windows-amd64": self._combined_case(
                "windows-amd64", "windows/scenarios/case-a-light.png", "b" * 64, receipt
            ),
        }
        combined = review_cases.combine_platform_manifests(manifests)
        markdown = review_cases.render_cross_platform_markdown(combined)
        metadata = markdown.split("## Observed artifact receipt metadata", 1)[1].split("## Boards and machine data", 1)[0]
        self.assertIn("1152", metadata)
        self.assertIn("Accounts search input", metadata)
        self.assertIn("abc123", metadata)
        self.assertIn("diff-hash", metadata)
        self.assertIn("binary-hash", metadata)
        self.assertIn("unknown", metadata)
        self.assertNotIn("registry only", metadata)


if __name__ == "__main__":
    unittest.main()
