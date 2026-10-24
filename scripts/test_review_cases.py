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
        self.assertFalse(any(case["review_status"] == "unreviewed" for case in cases.values()))

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
