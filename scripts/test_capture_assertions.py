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
        self.png.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\0" * 8 + struct.pack(">II", 360, 720))
        self.window = {"title": "Block · Accounts", "width": 360, "height": 720}
        self.expected = dict(title="Block · Accounts", viewport=(360, 720), required=["Account details", "PageSeed"],
                             observations=[{"text": "Account details"}, {"text": "PageSeed"}])

    def test_correct_capture_passes(self):
        self.assertTrue(validate(self.png, self.window, **self.expected)["passed"])

    def test_home_or_games_title_is_rejected(self):
        for title in ("Block", "Block · Games"):
            with self.subTest(title=title), self.assertRaisesRegex(ValueError, "Wrong page title"):
                validate(self.png, {**self.window, "title": title}, **self.expected)

    def test_wrong_viewport_is_rejected(self):
        for width, height in ((432, 768), (1152, 720), (402, 762)):
            with self.subTest(width=width, height=height), self.assertRaisesRegex(ValueError, "Wrong logical viewport"):
                validate(self.png, {**self.window, "width": width, "height": height}, **self.expected)

    def test_compact_pixels_must_have_the_content_aspect_and_supported_scale(self):
        for width, height, accepted in ((720, 1440, True), (1152, 720, False), (720, 720, False), (1080, 2160, False)):
            with self.subTest(width=width, height=height):
                self.png.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\0" * 8 + struct.pack(">II", width, height))
                if accepted:
                    self.assertEqual(validate(self.png, self.window, **self.expected)["dpr"], 2)
                else:
                    with self.assertRaisesRegex(ValueError, "does not match logical viewport"):
                        validate(self.png, self.window, **self.expected)

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
        assertions = validate(self.png, self.window, **self.expected, contract_revision="compact-ui-20261105")
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
        self.assertEqual(second_receipt["assertions"]["contract_revision"], "compact-ui-20261105")


if __name__ == "__main__":
    unittest.main()
