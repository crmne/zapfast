#!/usr/bin/env bash
# Installs a source build for the current user: the binary, the icon, and a
# launcher entry that opens ZapFast with no terminal.
#
# Usage: packaging/install-user.sh [binary] [prefix]
#   binary  the built executable (default: target/release/zapfast)
#   prefix  the install prefix   (default: $PREFIX, else ~/.local with data
#           in $XDG_DATA_HOME when it is set)
#
# The desktop file in packaging/ names its binary as `Exec=zapfast %u`, which is
# right for a package: a package manager installs the binary to /usr/bin,
# where every session finds it on PATH. A user install lands in ~/.local/bin,
# which a graphical session often does not put on PATH, so `Exec=zapfast`
# would find nothing and the launcher entry would do nothing when clicked.
# This script therefore writes the installed entry with the full path to the
# binary it just installed, computed here at install time. No machine's path
# is written into the repository.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo=$(dirname -- "$here")

binary=${1:-"$repo/target/release/zapfast"}
prefix=${2:-"${PREFIX:-}"}
if [[ -n "$prefix" ]]; then
  # An explicit prefix owns everything, data included, and is made absolute
  # because a launcher cannot resolve a relative Exec path.
  mkdir -p "$prefix"
  prefix=$(cd -- "$prefix" && pwd)
  data_dir="$prefix/share"
else
  prefix="$HOME/.local"
  data_dir="${XDG_DATA_HOME:-$prefix/share}"
fi
bin_dir="$prefix/bin"
apps_dir="$data_dir/applications"
icons_dir="$data_dir/icons/hicolor/scalable/apps"

if [[ ! -x "$binary" ]]; then
  echo "No built binary at $binary; build it first: cargo build --release --locked" >&2
  exit 1
fi

installed="$bin_dir/zapfast"
install -Dm755 "$binary" "$installed"
install -Dm644 "$here/icons/zapfast.svg" "$icons_dir/zapfast.svg"
mkdir -p "$apps_dir"
# The one line that changes: `Exec=zapfast %u` uses the path just installed,
# so the entry works whether or not ~/.local/bin is on the session's PATH.
#
# A simple absolute path stays unquoted: xdg-open's generic desktop handler
# treats quotes as part of the command name. Paths with reserved characters
# are quoted and escaped the way a Desktop Entry's Exec key wants, the
# same rules src/autostart.rs applies to the tray entry, because a home
# directory like `/home/alice/Zap Fast` is valid: unquoted, the launcher would
# read that as an executable plus an argument and the entry would do nothing.
# `"`, `` ` ``, `$` and `\` are backslash-escaped, and a literal `%` is doubled.
#
# The path reaches awk through the environment, not `awk -v`: `-v` expands
# backslash escapes in the value before the script sees it, so a prefix
# containing `\n` would gain a newline and the Exec would no longer match
# where the binary actually landed. ENVIRON values are used verbatim.
EXEC_PATH="$installed" awk '
  function quote(path,   out, i, c) {
    out = "\""
    for (i = 1; i <= length(path); i++) {
      c = substr(path, i, 1)
      if (c == "\"" || c == "`" || c == "$" || c == "\\") out = out "\\"
      if (c == "%") out = out "%"
      out = out c
    }
    return out "\""
  }
  /^Exec=/ {
    path = ENVIRON["EXEC_PATH"]
    if (path !~ /^\/[A-Za-z0-9_.\/-]+$/) path = quote(path)
    print "Exec=" path " %u"
    next
  }
  { print }
' "$here/applications/zapfast.desktop" > "$apps_dir/zapfast.desktop"

# desktop-file-validate rejects the `\\` that a literal backslash in a quoted
# Exec must use, so a prefix containing one is installed without that check
# rather than failing the install on a valid entry.
if command -v desktop-file-validate >/dev/null 2>&1 && [[ "$installed" != *\\* ]]; then
  desktop-file-validate "$apps_dir/zapfast.desktop"
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$apps_dir" 2>/dev/null || true
fi

echo "Installed $installed"
echo "Installed $apps_dir/zapfast.desktop"
echo "ZapFast now opens from the application launcher."
