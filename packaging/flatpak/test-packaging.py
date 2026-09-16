#!/usr/bin/env python3
"""Check manifest parity and revision-pinned generation without network access."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
APP_ID = "rocks.zapfast.ZapFast"


class FlatpakPackaging(unittest.TestCase):
    def test_source_and_bundle_have_the_same_sandbox(self):
        source = yaml.safe_load((HERE / f"{APP_ID}.yml").read_text())
        bundle = yaml.safe_load((HERE / f"{APP_ID}.bundle.yml").read_text())
        for key in ("id", "runtime", "runtime-version", "sdk", "command", "finish-args"):
            self.assertEqual(source[key], bundle[key], key)
        self.assertIn("--persist=.local/state", source["finish-args"])
        self.assertIn("--talk-name=org.freedesktop.secrets", source["finish-args"])
        self.assertFalse(any(arg.startswith("--filesystem=") for arg in source["finish-args"]))
        meta = ET.parse(HERE / f"{APP_ID}.metainfo.xml").getroot()
        self.assertEqual(meta.findtext("id"), APP_ID)
        self.assertEqual(meta.findtext("launchable"), f"{APP_ID}.desktop")

    def test_generation_uses_the_requested_revisions_lockfile(self):
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        lock = subprocess.check_output(["git", "show", f"{revision}:Cargo.lock"], cwd=ROOT)
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            stub = directory / "generator.py"
            stub.write_text('''import hashlib, json, pathlib, sys
lock = pathlib.Path(sys.argv[1]).read_bytes()
pathlib.Path(sys.argv[sys.argv.index("-o") + 1]).write_text(json.dumps({"lock_sha256": hashlib.sha256(lock).hexdigest()}))
''')
            output = directory / "output"
            subprocess.run([str(HERE / "flathub.sh"), revision, str(output)], cwd=ROOT,
                           env={**os.environ, "FLATPAK_CARGO_GENERATOR": str(stub)}, check=True)
            sources = json.loads((output / "cargo-sources.json").read_text())
            self.assertEqual(sources["lock_sha256"], hashlib.sha256(lock).hexdigest())
            manifest = yaml.safe_load((output / f"{APP_ID}.yml").read_text())
            self.assertEqual(manifest["modules"][0]["sources"][0]["commit"], revision)
            self.assertEqual(manifest["modules"][0]["sources"][0]["type"], "git")
            self.assertTrue((output / f"{APP_ID}.metainfo.xml").is_file())


if __name__ == "__main__":
    unittest.main()
