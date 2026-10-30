#!/usr/bin/env python3
"""Put the speech runtime beside a ZapFast build.

    python3 packaging/onnxruntime.py TARGET DESTINATION

TARGET is a Rust target triple, or universal-apple-darwin for the macOS
universal app. DESTINATION receives the ONNX Runtime library ZapFast loads
(onnxruntime.dll, libonnxruntime.dylib or libonnxruntime.so) and its license
and notices. The library comes from Microsoft's Microsoft.ML.OnnxRuntime
package on NuGet, pinned with its SHA-256 in flatpak/onnxruntime-sources.json,
which Flatpak and Nix read too, and each library is checked against
onnxruntime.sha256, which build.rs reads when it carries one inside ZapFast
(ZAPFAST_ONNXRUNTIME_EMBED). The package is kept under target/onnxruntime.
"""

import hashlib
import json
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
# Rust target => NuGet runtime identifiers, merged into one file when several.
TARGETS = {
    "x86_64-unknown-linux-gnu": ["linux-x64"],
    "aarch64-unknown-linux-gnu": ["linux-arm64"],
    "x86_64-pc-windows-msvc": ["win-x64"],
    "aarch64-pc-windows-msvc": ["win-arm64"],
    "aarch64-apple-darwin": ["osx-arm64"],
    "x86_64-apple-darwin": ["osx-x64"],
    "universal-apple-darwin": ["osx-arm64", "osx-x64"],
}
NOTICES = {"LICENSE": "onnxruntime-LICENSE.txt", "ThirdPartyNotices.txt": "onnxruntime-ThirdPartyNotices.txt"}


def library(rid):
    if rid.startswith("win-"):
        return "onnxruntime.dll"
    if rid.startswith("osx-"):
        return "libonnxruntime.dylib"
    return "libonnxruntime.so"


def source():
    [pin] = json.loads((HERE / "flatpak" / "onnxruntime-sources.json").read_text())
    return pin["url"], pin["sha256"]


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def package(cache):
    url, expected = source()
    path = cache / f"{expected}.nupkg"
    if path.is_file() and sha256(path) == expected:
        return path
    cache.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(".part")
    with urllib.request.urlopen(url, timeout=60) as response, open(partial, "wb") as file:
        while block := response.read(1 << 20):
            file.write(block)
    if sha256(partial) != expected:
        partial.unlink()
        sys.exit(f"{url} does not match its pinned SHA-256")
    partial.replace(path)
    return path


def pinned(hashes=None):
    """The SHA-256 of each library, by its path inside the package."""
    text = hashes or (HERE / "onnxruntime.sha256").read_text()
    return {name: digest for digest, name in (line.split() for line in text.splitlines() if line.strip())}


def extract(archive, rid, hashes):
    name = f"runtimes/{rid}/native/{library(rid)}"
    data = archive.read(name)
    if hashlib.sha256(data).hexdigest() != hashes.get(name):
        sys.exit(f"{name} does not match onnxruntime.sha256")
    return data


def place(target, destination, nupkg, hashes=None):
    rids = TARGETS[target]
    hashes = pinned(hashes)
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(nupkg) as archive:
        for name, output in NOTICES.items():
            (destination / output).write_bytes(archive.read(name))
        if len(rids) == 1:
            rid = rids[0]
            (destination / library(rid)).write_bytes(extract(archive, rid, hashes))
            return
        with tempfile.TemporaryDirectory(dir=destination) as work:
            slices = []
            for rid in rids:
                path = Path(work) / rid
                path.write_bytes(extract(archive, rid, hashes))
                slices.append(str(path))
            subprocess.run(["lipo", "-create", *slices, "-output", str(destination / library(rids[0]))], check=True)


def main(arguments):
    if len(arguments) != 2 or arguments[0] not in TARGETS:
        sys.exit(f"usage: onnxruntime.py {{{','.join(TARGETS)}}} DESTINATION")
    target, destination = arguments
    place(target, Path(destination), package(ROOT / "target" / "onnxruntime"))


if __name__ == "__main__":
    main(sys.argv[1:])
