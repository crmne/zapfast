#!/usr/bin/env python3
"""Check manifest parity and revision-pinned generation without network access."""
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
APP_ID = "rocks.zapfast.ZapFast"


class FlatpakPackaging(unittest.TestCase):
    def test_native_runtime_sources_are_pinned_for_both_linux_architectures(self):
        sources = json.loads((HERE / "onnxruntime-sources.json").read_text())
        self.assertEqual(len(sources), 2)
        self.assertEqual({tuple(source["only-arches"]) for source in sources},
                         {("x86_64",), ("aarch64",)})
        for source in sources:
            arch = source["only-arches"][0]
            self.assertEqual(source["type"], "file")
            self.assertEqual(source["url"],
                             f"https://cdn.pyke.io/0/pyke:ort-rs/ms@1.31.0/{arch}-unknown-linux-gnu.tar.lzma2")
            self.assertRegex(source["sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(source["dest"], "onnxruntime")
            self.assertEqual(source["dest-filename"], "onnxruntime.tar.lzma2")
        # Updating ort-sys requires rechecking its runtime asset table, not
        # silently using native libraries selected by an older release.
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        versions = [package["version"] for package in lock["package"]
                    if package["name"] == "ort-sys"]
        self.assertEqual(versions, ["2.0.0-rc.15"])
        nix = (ROOT / "flake.nix").read_text()
        self.assertIn("builtins.readFile ./packaging/flatpak/onnxruntime-sources.json", nix)
        self.assertIn('ORT_SKIP_DOWNLOAD = "1";', nix)

    def test_runtime_downloader_uses_rustls_not_system_openssl(self):
        dependencies = tomllib.loads((ROOT / "Cargo.toml").read_text())["dependencies"]
        parakeet = dependencies["parakeet-rs"]
        self.assertFalse(parakeet["default-features"])
        self.assertNotIn("ort-defaults", parakeet["features"])
        ort = dependencies["ort"]
        self.assertEqual(ort["version"], "=2.0.0-rc.15")
        self.assertFalse(ort["default-features"])
        self.assertIn("tls-rustls", ort["features"])
        self.assertNotIn("tls-native", ort["features"])

    def test_build_unpacks_the_verified_runtime_before_offline_cargo(self):
        manifest = yaml.safe_load((HERE / f"{APP_ID}.yml").read_text())
        self.assertEqual(manifest["build-options"]["env"]["ORT_SKIP_DOWNLOAD"], "1")
        self.assertEqual(manifest["build-options"]["env"]["ORT_LIB_LOCATION"],
                         "/run/build/zapfast/onnxruntime")
        self.assertNotIn("--share=network", manifest["build-options"].get("build-args", []))
        module = manifest["modules"][0]
        self.assertIn("onnxruntime-sources.json", module["sources"])
        data = b"synthetic static-library fixture"
        tar = io.BytesIO()
        with tarfile.open(fileobj=tar, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            entry = tarfile.TarInfo("libonnxruntime.a")
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
        compressed = lzma.compress(tar.getvalue(), format=lzma.FORMAT_RAW,
                                   filters=[{"id": lzma.FILTER_LZMA2, "dict_size": 1 << 26}])
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            runtime = directory / "onnxruntime"
            runtime.mkdir()
            (runtime / "onnxruntime.tar.lzma2").write_bytes(compressed)
            for command in module["build-commands"]:
                if command.startswith("cargo "):
                    self.assertIn("--offline", command)
                    break
                subprocess.run(["bash", "-euo", "pipefail", "-c", command],
                               cwd=directory, check=True)
            else:
                self.fail("the manifest must still build with offline Cargo")
            self.assertEqual((runtime / "libonnxruntime.a").read_bytes(), data)

    def test_source_and_bundle_have_the_same_sandbox(self):
        source = yaml.safe_load((HERE / f"{APP_ID}.yml").read_text())
        bundle = yaml.safe_load((HERE / f"{APP_ID}.bundle.yml").read_text())
        for key in ("id", "runtime", "runtime-version", "sdk", "command", "finish-args"):
            self.assertEqual(source[key], bundle[key], key)
        self.assertIn("--persist=.local/state", source["finish-args"])
        self.assertIn("--talk-name=org.freedesktop.secrets", source["finish-args"])
        self.assertIn("--talk-name=org.freedesktop.Notifications", source["finish-args"])
        # Video calls read the camera through Video4Linux.
        self.assertIn("--device=all", source["finish-args"])
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
            self.assertEqual((output / "onnxruntime-sources.json").read_bytes(),
                             (HERE / "onnxruntime-sources.json").read_bytes())


if __name__ == "__main__":
    unittest.main()
