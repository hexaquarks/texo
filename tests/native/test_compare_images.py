import tempfile
import unittest
from pathlib import Path

from PIL import Image

from compare_images import compare_screenshot, write_review


class ScreenshotReviewTests(unittest.TestCase):
    def test_missing_identical_changed_and_resized_baselines(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            current = root / "current.png"
            baseline = root / "baseline.png"
            difference = root / "diff.png"
            Image.new("RGB", (8, 8), "black").save(current)

            self.assertEqual(compare_screenshot(current, baseline, difference), "No baseline yet")
            Image.new("RGB", (8, 8), "black").save(baseline)
            self.assertEqual(compare_screenshot(current, baseline, difference), "Unchanged")
            self.assertFalse(difference.exists())

            Image.new("RGB", (8, 8), "white").save(baseline)
            self.assertTrue(compare_screenshot(current, baseline, difference).startswith("Changed"))
            with Image.open(difference) as image:
                self.assertEqual(image.getpixel((0, 0)), (255, 255, 255))

            Image.new("RGB", (12, 8), "white").save(baseline)
            self.assertTrue(compare_screenshot(current, baseline, difference).startswith("Dimensions changed"))
            self.assertFalse(difference.exists())

    def test_review_contains_images_and_flags_removed_views(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            current = root / "current"
            baseline = root / "baseline"
            output = root / "review"
            current.mkdir()
            baseline.mkdir()
            Image.new("RGB", (8, 8), "black").save(current / "startup.png")
            Image.new("RGB", (8, 8), "white").save(baseline / "startup.png")
            Image.new("RGB", (8, 8), "white").save(baseline / "settings.png")

            write_review(current, baseline, output)

            report = (output / "index.html").read_text()
            self.assertIn("diff-startup.png", report)
            self.assertIn("Missing current screenshots: settings.png", report)
            self.assertTrue((output / "current-startup.png").is_file())
            self.assertTrue((output / "baseline-startup.png").is_file())

    def test_no_current_screenshots_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "No screenshots"):
                write_review(root / "missing", root / "baseline", root / "review")


if __name__ == "__main__":
    unittest.main()
