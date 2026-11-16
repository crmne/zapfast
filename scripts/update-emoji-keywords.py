#!/usr/bin/env python3
"""Regenerates assets/emoji/keywords-en.tsv from Unicode CLDR annotations.

Usage: scripts/update-emoji-keywords.py [CLDR release tag, default below]

Each line is an emoji without its U+FE0F selectors, a tab, and its English
search keywords in lower case separated by " | ". Skin tone variants are left
out: the picker searches base emoji only.
"""

import sys
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

RELEASE = sys.argv[1] if len(sys.argv) > 1 else "release-49"
URL = "https://raw.githubusercontent.com/unicode-org/cldr/{}/common/{}/en.xml"
OUT = Path(__file__).resolve().parent.parent / "assets/emoji/keywords-en.tsv"

lines = []
for part in ("annotations", "annotationsDerived"):
    with urllib.request.urlopen(URL.format(RELEASE, part)) as response:
        root = ET.parse(response).getroot()
    for node in root.iter("annotation"):
        emoji = node.get("cp").replace("️", "")
        if node.get("type") == "tts" or any(0x1F3FB <= ord(c) <= 0x1F3FF for c in emoji):
            continue
        lines.append(f"{emoji}\t{node.text.lower()}\n")
OUT.write_text("".join(lines), encoding="utf-8")
