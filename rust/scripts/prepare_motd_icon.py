#!/usr/bin/env python3
"""Regenerate the embedded 64x64 MOTD icon from the repository logo (Pillow)."""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parents[1]
image = Image.open(root.parent / "EaglerXPumpkin.png").convert("RGBA")
image.thumbnail((64, 64), Image.Resampling.LANCZOS)
icon = Image.new("RGBA", (64, 64))
icon.paste(image, ((64 - image.width) // 2, (64 - image.height) // 2))
icon.save(root / "crates/eaglerx-pumpkin/assets/motd-icon.png")
