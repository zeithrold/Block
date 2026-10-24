"""Wrong page/geometry/content must fail even if the candidate file exists."""
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from capture_assertions import publish, sha256, validate


class CaptureAssertionsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.png = Path(self.temp.name) / "accounts.png"
        self.png.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\0" * 8 + struct.pack(">II", 960, 600))
        self.window = {"title": "Block · Accounts", "width": 960, "height": 600}
        self.expected = dict(title="Block · Accounts", viewport=(960, 600), required=["Account details", "PageSeed"],
                             observations=[{"text": "Account details"}, {"text": "PageSeed"}])

    def test_correct_capture_passes(self):
        self.assertTrue(validate(self.png, self.window, **self.expected)["passed"])

    def test_home_or_games_title_is_rejected(self):
        for title in ("Block", "Block · Games"):
            with self.subTest(title=title), self.assertRaisesRegex(ValueError, "Wrong page title"):
                validate(self.png, {**self.window, "title": title}, **self.expected)

    def test_wrong_viewport_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Wrong logical viewport"):
            validate(self.png, {**self.window, "width": 432, "height": 768}, **self.expected)

    def test_correct_title_without_unique_content_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Page content assertions"):
            validate(self.png, self.window, **{**self.expected, "observations": [{"text": "Games PageSeed"}]})

    def test_ocr_ellipsis_style_does_not_hide_a_loading_state(self):
        assertions = validate(self.png, self.window, **{**self.expected,
            "required": ["Loading official builds…"], "observations": [{"text": "Loading official builds..."}]})
        self.assertTrue(assertions["passed"])

    def test_replacing_twice_preserves_each_previous_image_and_receipt(self):
        root = Path(self.temp.name)
        destination = root / "target/parity-shots/native/accounts.png"
        destination.parent.mkdir(parents=True)
        original = self.png.read_bytes() + b"original"
        first = self.png.read_bytes() + b"first replacement"
        destination.write_bytes(original)
        assertions = validate(self.png, self.window, **self.expected)
        with patch("capture_assertions.ROOT", root):
            self.png.write_bytes(first)
            first_receipt = publish(self.png, destination, {}, assertions)
            baseline = root / first_receipt["replaced_original"]["path"]
            first_receipt_hash = sha256(destination.with_suffix(".capture.json"))
            self.png.write_bytes(self.png.read_bytes() + b"second replacement")
            second_receipt = publish(self.png, destination, {}, assertions)
        self.assertEqual(baseline.read_bytes(), original)
        self.assertEqual((root / second_receipt["replaced_original"]["path"]).read_bytes(), first)
        self.assertEqual(second_receipt["baseline_original"]["path"], str(baseline.relative_to(root)))
        self.assertEqual(sha256(root / second_receipt["replaced_receipt"]["path"]), first_receipt_hash)


if __name__ == "__main__":
    unittest.main()
