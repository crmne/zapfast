#!/bin/bash
# Builds, signs and publishes a release of this fork, which installed copies
# then update to by themselves (src/updates.rs):
#
#   scripts/release-fork.sh <version> [notes.md]
#
# It sets <version> in Cargo.toml and Cargo.lock, builds the Apple silicon
# disk image and the Windows x64 portable zip on this Mac, writes
# checksums.txt, signs it with the Ed25519 key the app trusts
# (assets/update-public-key.hex), and publishes the GitHub release
# v<version> on the fork with gh. PUBLISH=0 stops before publishing, leaving
# the files in $OUT.
#
# The private key is ~/.config/zapfast-release/update-signing-key.pem (or
# $ZAPFAST_SIGNING_KEY), backed up with rsync to /Volumes/storage/Keys/zapfast/,
# which is used when the first is missing. Copies already installed accept
# only releases signed with it.
set -euo pipefail

version="${1:?usage: scripts/release-fork.sh <version> [notes.md]}"
notes="${2:-}"
repo=iddhi-sulakshana/zapfast
key="${ZAPFAST_SIGNING_KEY:-$HOME/.config/zapfast-release/update-signing-key.pem}"
# The backup on the storage volume, when the copy in ~/.config is gone.
backup_key=/Volumes/storage/Keys/zapfast/update-signing-key.pem
[ -r "$key" ] || [ -n "${ZAPFAST_SIGNING_KEY:-}" ] || key="$backup_key"
llvm_mingw="${LLVM_MINGW:-/storage/tools/llvm-mingw-20261020-ucrt-macos-universal}"
cmake_bin="${CMAKE_BIN:-/storage/venvs/cmake/bin}"
out="${OUT:-/storage/builds/zapfast/release-v$version}"
publish="${PUBLISH:-1}"

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

die() {
    echo "release-fork: $*" >&2
    exit 1
}

# A pre-release (1.2.3-rc.1) is never offered to installed copies.
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "the version must be X.Y.Z"
[ -r "$key" ] || die "no signing key at $key"
[ -z "$notes" ] || [ -r "$notes" ] || die "no notes file at $notes"
[ -x "$llvm_mingw/bin/x86_64-w64-mingw32-clang" ] || die "no llvm-mingw at $llvm_mingw"
openssl=/opt/homebrew/bin/openssl
[ -x "$openssl" ] || openssl=$(command -v openssl)
"$openssl" version | grep -Eq '^OpenSSL [3-9]' || die "Ed25519 signing needs OpenSSL 3 or newer"

# Only the key the app embeds makes a release installed copies accept.
public=$("$openssl" pkey -in "$key" -pubout -outform DER | tail -c 32 | xxd -p -c 64)
[ "$public" = "$(tr -d '[:space:]' < assets/update-public-key.hex)" ] ||
    die "the signing key does not match assets/update-public-key.hex"

if [ "$publish" = 1 ]; then
    gh auth status >/dev/null 2>&1 || die "gh is not signed in (gh auth login)"
    latest=$(gh release view -R "$repo" --json tagName -q .tagName 2>/dev/null || true)
    if [ -n "$latest" ]; then
        newest=$(printf '%s\n%s\n' "${latest#v}" "$version" | sort -V | tail -1)
        { [ "$newest" = "$version" ] && [ "${latest#v}" != "$version" ]; } ||
            die "v$version is not newer than the latest release, $latest"
    fi
fi

# The version every file reports: the updater checks the downloaded app's
# --version and its bundle's version against the release's.
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ "$current" != "$version" ]; then
    perl -0pi -e "s/^version = \"\\Q$current\\E\"/version = \"$version\"/m" Cargo.toml
    perl -0pi -e "s/(name = \"zapfast\"\\nversion = )\"[^\"]*\"/\${1}\"$version\"/" Cargo.lock
    echo "Set the version to $version in Cargo.toml and Cargo.lock (was $current)."
fi

export PATH="$cmake_bin:$PATH" CMAKE_POLICY_VERSION_MINIMUM=3.5
rm -rf "$out"
stage="$out/stage"
mkdir -p "$stage/mac"

echo "Building for macOS..."
cargo build --locked --release --bin zapfast
packaging/macos/bundle.sh target/release/zapfast "$stage/mac/ZapFast.app" "$version" >/dev/null
ln -s /Applications "$stage/mac/Applications"
dmg="zapfast-v$version-macos-arm64.dmg"
hdiutil create -volname ZapFast -srcfolder "$stage/mac" -format UDZO "$out/$dmg" >/dev/null
hdiutil verify "$out/$dmg" >/dev/null

echo "Building for Windows..."
win_target=x86_64-pc-windows-gnullvm
# libc++ and libunwind linked in statically, so the exe needs no DLLs of
# llvm-mingw's: rustc forces -Bdynamic, so a folder holding only their
# static archives is put first on the search path.
static="$root/.cache/winstatic"
mkdir -p "$static"
cp "$llvm_mingw/x86_64-w64-mingw32/lib/libc++.a" "$llvm_mingw/x86_64-w64-mingw32/lib/libunwind.a" "$static/"
(
    export PATH="$llvm_mingw/bin:$PATH" \
        CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang \
        CXX_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang++ \
        AR_x86_64_pc_windows_gnullvm=llvm-ar \
        RANLIB_x86_64_pc_windows_gnullvm=llvm-ranlib \
        CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang \
        CARGO_TARGET_DIR=target/windows
    cargo rustc --locked --release --target "$win_target" --bin zapfast -- -L "native=$static"
)
exe="target/windows/$win_target/release/zapfast.exe"
if "$llvm_mingw/bin/llvm-readobj" --coff-imports "$exe" | grep -Eqi 'libc\+\+|libunwind'; then
    die "the Windows exe still needs llvm-mingw's DLLs"
fi
"$llvm_mingw/bin/llvm-readobj" --coff-resources "$exe" | grep -q GROUP_ICON ||
    die "the Windows exe has no icon"
# The updater names Windows downloads after the msvc target whatever built
# them, and installs only a copy with the portable marker beside it.
win="zapfast-v$version-x86_64-pc-windows-msvc"
mkdir -p "$stage/$win"
cp "$exe" packaging/zapfast-portable.txt README.md LICENSE THIRD-PARTY-NOTICES.md "$stage/$win/"
cp assets/sounds/README.md "$stage/$win/SOUNDS.md"
(cd "$stage" && zip -qr "$out/$win.zip" "$win")

echo "Signing..."
(cd "$out" && shasum -a 256 "$dmg" "$win.zip" > checksums.txt)
"$openssl" pkeyutl -sign -rawin -inkey "$key" -in "$out/checksums.txt" -out "$out/checksums.txt.sig"
[ "$(wc -c < "$out/checksums.txt.sig" | tr -d ' ')" = 64 ] || die "the signature is not 64 bytes"
"$openssl" pkey -in "$key" -pubout -out "$stage/public.pem"
"$openssl" pkeyutl -verify -rawin -pubin -inkey "$stage/public.pem" \
    -in "$out/checksums.txt" -sigfile "$out/checksums.txt.sig" >/dev/null ||
    die "the signature does not verify"
rm -rf "$stage"

if [ "$publish" != 1 ]; then
    echo "Built and signed in $out (not published)."
    exit 0
fi

# The tag goes on this commit when GitHub has it, else on the fork's main.
head=$(git rev-parse HEAD)
if git branch -r --contains "$head" 2>/dev/null | grep -q 'origin/'; then
    target=$head
else
    target=main
    echo "warning: this commit is not pushed; v$version will be tagged on the fork's main" >&2
fi
if ! git diff --quiet HEAD -- . ':!Cargo.toml' ':!Cargo.lock'; then
    echo "warning: uncommitted changes are in the build but not in the tagged source" >&2
fi
args=(--title "ZapFast v$version" --target "$target")
if [ -n "$notes" ]; then
    args+=(--notes-file "$notes")
else
    args+=(--notes "ZapFast v$version for macOS (Apple silicon) and Windows (x64).")
fi
gh release create "v$version" -R "$repo" "${args[@]}" \
    "$out/$dmg" "$out/$win.zip" "$out/checksums.txt" "$out/checksums.txt.sig"
echo "Published v$version: https://github.com/$repo/releases/tag/v$version"
