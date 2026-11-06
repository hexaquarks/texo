"""Compare terminal screenshots with the last successful main run for human review."""

import argparse
import html
import shutil
from pathlib import Path

from PIL import Image, ImageChops


def compare_screenshot(current: Path, baseline: Path, output: Path) -> str:
    """Save a difference image without treating intentional UI changes as failures."""
    output.unlink(missing_ok=True)
    if not baseline.is_file():
        return "No baseline yet"

    with Image.open(current) as current_image, Image.open(baseline) as baseline_image:
        current_rgb = current_image.convert("RGB")
        baseline_rgb = baseline_image.convert("RGB")

    if current_rgb.size != baseline_rgb.size:
        return f"Dimensions changed: {baseline_rgb.size} → {current_rgb.size}"

    difference = ImageChops.difference(current_rgb, baseline_rgb)
    if difference.getbbox() is None:
        return "Unchanged"

    difference.save(output)
    return "Changed — review the screenshots and difference image"


def write_review(current: Path, baseline: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    screenshots = sorted(current.glob("*.png"))
    if not screenshots:
        raise ValueError(f"No screenshots found in {current}")

    sections = []
    for screenshot in screenshots:
        reference = baseline / screenshot.name
        difference = output / f"diff-{screenshot.name}"
        status = compare_screenshot(screenshot, reference, difference)

        shutil.copy2(screenshot, output / f"current-{screenshot.name}")
        images = [("Current", f"current-{screenshot.name}")]
        if reference.is_file():
            shutil.copy2(reference, output / f"baseline-{screenshot.name}")
            images.append(("Baseline", f"baseline-{screenshot.name}"))
        if difference.is_file():
            images.append(("Difference", difference.name))

        figures = "\n".join(
            f'<figure><figcaption>{label}</figcaption>'
            f'<a href="{html.escape(name)}"><img src="{html.escape(name)}" alt="{label}"></a></figure>'
            for label, name in images
        )
        sections.append(
            f"<section><h2>{html.escape(screenshot.stem)}</h2>"
            f"<p>{html.escape(status)}</p>{figures}</section>"
        )
        print(f"{screenshot.name}: {status}")

    removed = sorted(path.name for path in baseline.glob("*.png") if not (current / path.name).exists())
    if removed:
        sections.append(f"<p>Missing current screenshots: {html.escape(', '.join(removed))}</p>")

    report = """<!doctype html>
<meta charset="utf-8">
<title>Terminal screenshot review</title>
<style>
body { font-family: sans-serif; margin: 2rem; }
figure { display: inline-block; vertical-align: top; margin: 0 1rem 1rem 0; width: 30%; }
img { width: 100%; }
</style>
<h1>Terminal screenshot review</h1>
<p>Pixel differences require human review. They do not change the smoke-test result.</p>
""" + "\n".join(sections)
    (output / "index.html").write_text(report, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("current", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()

    write_review(arguments.current, arguments.baseline, arguments.output)


if __name__ == "__main__":
    main()
