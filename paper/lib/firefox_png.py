"""Rasterize a manuscript figure page with headless Firefox, as the browser shows it.

    from firefox_png import render
    render(html_path, png_path, width)

Firefox screenshots its window, not the whole page, so the page is drawn in a
window taller than any figure and the white space below the figure is cut off.
The page must have a white background. Needs Firefox and Pillow.
"""
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops

# Taller than any manuscript figure at 2400 px wide.
WINDOW_HEIGHT = 6000
# White kept below the lowest drawn pixel.
BOTTOM_MARGIN = 24


def render(html_path, png_path, width):
    html_path = Path(html_path).resolve()
    png_path = Path(png_path).resolve()
    # A throwaway profile keeps the user's own Firefox profile untouched and
    # lets this run beside an open Firefox window.
    profile = Path(tempfile.mkdtemp(prefix="coxeter-figure-"))
    try:
        subprocess.run(
            ["firefox", "--headless", "--no-remote", "--profile", str(profile),
             "--screenshot", str(png_path), f"--window-size={width},{WINDOW_HEIGHT}", html_path.as_uri()],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
    finally:
        shutil.rmtree(profile, ignore_errors=True)
    image = Image.open(png_path).convert("RGB")
    white = Image.new("RGB", image.size, (255, 255, 255))
    box = ImageChops.difference(image, white).getbbox()
    if box is None:
        raise RuntimeError(f"{html_path.name} rendered as an empty page.")
    height = min(image.height, box[3] + BOTTOM_MARGIN)
    image.crop((0, 0, image.width, height)).save(png_path, dpi=(340, 340))
    return image.width, height
