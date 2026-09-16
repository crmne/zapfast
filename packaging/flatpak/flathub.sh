#!/usr/bin/env bash
# Prepare an immutable source manifest and its offline Cargo dependencies.
# Usage: flathub.sh vX.Y.Z /path/to/flathub-checkout
set -euo pipefail
revision=${1:?Supply a tag or commit}
out=${2:?Supply an output directory}
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
commit=$(git -C "$root" rev-parse --verify --end-of-options "$revision^{commit}")
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
git -C "$root" show "$commit:Cargo.lock" > "$work/Cargo.lock"
git -C "$root" show "$commit:Cargo.toml" > "$work/Cargo.toml"
# Pin the generator; do not execute mutable remote code from master.
generator=${FLATPAK_CARGO_GENERATOR:-$work/flatpak-cargo-generator.py}
if [[ -z ${FLATPAK_CARGO_GENERATOR:-} ]]; then
  curl --fail --location --retry 3 \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/de2225a6dee4818c1339b3cdbf29f90c471fcb7e/cargo/flatpak-cargo-generator.py \
    --output "$generator"
fi
mkdir -p "$out"
python3 "$generator" "$work/Cargo.lock" -o "$out/cargo-sources.json"
python3 - "$here" "$out" "$commit" "$work/Cargo.toml" <<'PY'
from pathlib import Path
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET
import yaml
here, out = map(Path, sys.argv[1:3])
commit = sys.argv[3]
version = tomllib.loads(Path(sys.argv[4]).read_text())["package"]["version"]
manifest = yaml.safe_load((here / "rocks.zapfast.ZapFast.yml").read_text())
manifest["modules"][0]["sources"][0] = {
    "type": "git", "url": "https://github.com/crmne/zapfast.git", "commit": commit
}
(out / "rocks.zapfast.ZapFast.yml").write_text(yaml.safe_dump(manifest, sort_keys=False))
meta = ET.parse(here / "rocks.zapfast.ZapFast.metainfo.xml")
releases = meta.getroot().find("releases")
releases.clear()
date = subprocess.check_output(["git", "-C", str(here), "show", "-s", "--format=%cs", commit], text=True).strip()
release = ET.SubElement(releases, "release", version=version, date=date)
ET.SubElement(release, "url").text = f"https://github.com/crmne/zapfast/releases/tag/v{version}"
ET.indent(meta, space="  ")
meta.write(out / "rocks.zapfast.ZapFast.metainfo.xml", encoding="UTF-8", xml_declaration=True)
PY
