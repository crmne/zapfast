#!/usr/bin/env python3
"""Regenerate the emoji picker's search keywords from Unicode CLDR.

Downloads the English annotations of a CLDR release (the argument, a tag of
github.com/unicode-org/cldr) and writes assets/emoji/keywords-en.tsv: one
emoji per line without its U+FE0F selectors, a tab, and its keywords in lower
case separated by " | ". Names and skin tone variants are left out.
"""
from pathlib import Path
import sys
import urllib.request
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'assets/emoji/keywords-en.tsv'
URL = 'https://raw.githubusercontent.com/unicode-org/cldr/{}/common/{}/en.xml'
SKIN_TONES = range(0x1F3FB, 0x1F400)


def annotations(release, part):
    with urllib.request.urlopen(URL.format(release, part)) as response:
        return ET.parse(response).getroot().iter('annotation')


def main():
    release = sys.argv[1] if len(sys.argv) > 1 else 'release-49'
    lines = []
    for part in ('annotations', 'annotationsDerived'):
        for node in annotations(release, part):
            emoji = node.get('cp').replace('️', '')
            # The tts entry is the name, which the emojis crate already has.
            if node.get('type') == 'tts' or any(ord(c) in SKIN_TONES for c in emoji):
                continue
            lines.append(f'{emoji}\t{node.text.lower()}\n')
    OUT.write_text(''.join(lines), encoding='utf-8')


if __name__ == '__main__':
    main()
