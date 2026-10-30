#!/usr/bin/env python3
"""Check manifest parity and revision-pinned generation without network access."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET
import zipfile

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
APP_ID = "rocks.zapfast.ZapFast"


class FlatpakPackaging(unittest.TestCase):
    def test_the_speech_runtime_comes_from_one_pinned_registry_package(self):
        [source] = json.loads((HERE / "onnxruntime-sources.json").read_text())
        self.assertEqual(source["type"], "file")
        self.assertRegex(source["url"],
                         r"^https://api\.nuget\.org/v3-flatcontainer/microsoft\.ml\.onnxruntime/")
        self.assertRegex(source["sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(source["dest-filename"], "onnxruntime.nupkg")
        nix = (ROOT / "flake.nix").read_text()
        self.assertIn("builtins.readFile ./packaging/flatpak/onnxruntime-sources.json", nix)
        self.assertIn("ZAPFAST_ONNXRUNTIME = ", nix)
        for recipe in ["zapfast", "zapfast-git"]:
            text = (ROOT / "packaging/arch" / recipe / "PKGBUILD.in").read_text()
            self.assertIn(source["url"], text, recipe)
            self.assertIn(source["sha256"], text, recipe)
        # Building fetches no native library: ZapFast loads it at run time.
        ort = tomllib.loads((ROOT / "Cargo.toml").read_text())["dependencies"]["ort"]
        self.assertIn("load-dynamic", ort["features"])
        self.assertNotIn("download-binaries", ort["features"])
        manifest = (HERE / f"{APP_ID}.yml").read_text()
        self.assertNotIn("ORT_", manifest + nix)

    def test_flatpak_installs_the_runtime_where_zapfast_looks(self):
        manifest = yaml.safe_load((HERE / f"{APP_ID}.yml").read_text())
        module = manifest["modules"][0]
        self.assertIn("onnxruntime-sources.json", module["sources"])
        [command] = [command for command in module["build-commands"] if "onnxruntime.nupkg" in command
                     and "FLATPAK_ARCH" in command]
        for arch, rid in [("x86_64", "linux-x64"), ("aarch64", "linux-arm64")]:
            with self.subTest(arch=arch), tempfile.TemporaryDirectory() as directory:
                directory = Path(directory)
                with zipfile.ZipFile(directory / "onnxruntime.nupkg", "w") as archive:
                    for native in ["linux-x64", "linux-arm64"]:
                        archive.writestr(f"runtimes/{native}/native/libonnxruntime.so", native)
                subprocess.run(["bash", "-euo", "pipefail", "-c", command.replace("/app/", f"{directory}/app/")],
                               cwd=directory, env={**os.environ, "FLATPAK_ARCH": arch}, check=True)
                self.assertEqual((directory / "app/lib/libonnxruntime.so").read_text(), rid)
        bundle = "\n".join(yaml.safe_load((HERE / f"{APP_ID}.bundle.yml").read_text())["modules"][0]["build-commands"])
        self.assertIn("libonnxruntime.so /app/lib/libonnxruntime.so", bundle)

    def test_the_runtime_helper_places_each_platforms_library(self):
        sys.dont_write_bytecode = True
        spec = importlib.util.spec_from_file_location("onnxruntime", ROOT / "packaging/onnxruntime.py")
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            nupkg = directory / "onnxruntime.nupkg"
            with zipfile.ZipFile(nupkg, "w") as archive:
                archive.writestr("LICENSE", "license")
                archive.writestr("ThirdPartyNotices.txt", "notices")
                for rid in ["linux-x64", "linux-arm64", "win-x64", "win-arm64", "osx-arm64", "osx-x64"]:
                    archive.writestr(f"runtimes/{rid}/native/{helper.library(rid)}", rid)
            for target, rid, name in [("x86_64-unknown-linux-gnu", "linux-x64", "libonnxruntime.so"),
                                      ("aarch64-pc-windows-msvc", "win-arm64", "onnxruntime.dll"),
                                      ("x86_64-apple-darwin", "osx-x64", "libonnxruntime.dylib")]:
                with self.subTest(target=target):
                    output = directory / target
                    helper.place(target, output, nupkg)
                    self.assertEqual((output / name).read_text(), rid)
                    self.assertEqual((output / "onnxruntime-LICENSE.txt").read_text(), "license")
                    self.assertEqual((output / "onnxruntime-ThirdPartyNotices.txt").read_text(), "notices")
        # Every release target has a runtime.
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for target in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
                       "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]:
            self.assertIn(f"target: {target}", workflow)
            self.assertIn(target, helper.TARGETS)
        self.assertIn("onnxruntime.py universal-apple-darwin", workflow)

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
