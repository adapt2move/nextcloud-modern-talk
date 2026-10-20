#!/usr/bin/env python3
"""Generate reproducible OFL static instances for GPUI's Linux font selection.

The original Manrope variable font remains unchanged. Some font-kit/cosmic-text
combinations select its default 200 weight without applying requested variation.
Run in the assigned VM with distro python3-fonttools, record its exact version,
then copy the generated assets back. No build-time font tooling is needed by users.
"""
import hashlib
import json
from pathlib import Path
import sys
import fontTools
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

source = Path(__file__).resolve().parents[2] / "apps/desktop/assets/fonts/Manrope-Variable.ttf"
out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
manifest = {"source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "fonttools": fontTools.__version__, "license": "OFL-1.1", "instances": []}
for weight, style in [(400, "Regular"), (600, "SemiBold"), (700, "Bold")]:
    font = TTFont(source, recalcTimestamp=False)
    font = instantiateVariableFont(font, {"wght": weight}, inplace=True, updateFontNames=True)
    font.recalcTimestamp = False
    for platform, encoding, language in [(3, 1, 0x409), (1, 0, 0)]:
        for name_id, value in [(1, "Manrope"), (2, style), (4, "Manrope " + style), (6, "Manrope-" + style), (16, "Manrope"), (17, style)]:
            font["name"].setName(value, name_id, platform, encoding, language)
    path = out / ("Manrope-" + style + ".ttf")
    font.save(path, reorderTables=True)
    manifest["instances"].append({"file": path.name, "weight": weight, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
(out / "static-manrope-provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
