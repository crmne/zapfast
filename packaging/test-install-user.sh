#!/usr/bin/env bash
# Tests packaging/install-user.sh without touching the real home directory.
#
# The interesting cases are prefixes that contain a space, a `%`, or a backslash,
# which are all valid home directories: the generated `Exec=` must be a quoted,
# escaped Desktop Entry value, or the launcher reads the path as an executable
# plus an argument (or as a different path) and the entry does nothing when
# clicked.
#
# Usage: bash packaging/test-install-user.sh
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

fake="$work/zapfast-fake"
cat > "$fake" <<'SCRIPT'
#!/bin/sh
if [ -n "${ZAPFAST_TEST_ARGS:-}" ]; then
  printf '%s\n' "$@" > "$ZAPFAST_TEST_ARGS"
fi
exit 0
SCRIPT
chmod +x "$fake"

# The Desktop Entry Exec escaping, mirroring src/autostart.rs: a backslash is
# doubled first, then `"`, `` ` ``, `$` and `\` are backslash-escaped, and a
# literal `%` is doubled. Written here so the test states the expected bytes
# rather than reading them back from the script under test.
escape_exec() {
  local path=$1
  path=${path//\\/\\\\}
  path=${path//\"/\\\"}
  path=${path//\`/\\\`}
  path=${path//\$/\\$}
  path=${path//%/%%}
  printf '%s' "$path"
}

check_prefix() {
  local prefix="$1"
  local style="${2:-quoted}"
  mkdir -p "$prefix"
  bash "$script_dir/install-user.sh" "$fake" "$prefix" >/dev/null
  local entry="$prefix/share/applications/zapfast.desktop"
  test -s "$entry"
  grep -qxF 'MimeType=x-scheme-handler/whatsapp;' "$entry"
  test -s "$prefix/share/icons/hicolor/scalable/apps/zapfast.svg"

  local expected
  if [[ "$style" == plain ]]; then
    expected=$(printf 'Exec=%s %%u' "${prefix}/bin/zapfast")
  else
    expected=$(printf 'Exec="%s" %%u' "$(escape_exec "${prefix}/bin/zapfast")")
  fi
  grep -qxF "$expected" "$entry" || {
    echo "unexpected Exec for prefix: $prefix" >&2
    grep '^Exec=' "$entry" >&2
    echo "expected: $expected" >&2
    exit 1
  }

  # desktop-file-validate rejects the spec-correct doubled backslash in a
  # quoted Exec, so it is only consulted for paths without one. The escaping is
  # still checked above, and mirrors src/autostart.rs.
  if command -v desktop-file-validate >/dev/null 2>&1 && [[ "$prefix" != *\\* ]]; then
    desktop-file-validate "$entry"
  fi
}

check_prefix "$work/with space"
check_prefix "$work/standard" plain
uri='whatsapp://send/?phone=15550100123&text=Hola%2C+informaci%C3%B3n+%26+precio'
if command -v gio >/dev/null 2>&1; then
  ZAPFAST_TEST_ARGS="$work/launched-args" gio launch \
    "$work/with space/share/applications/zapfast.desktop" "$uri"
  for attempt in {1..50}; do
    [[ -f "$work/launched-args" ]] && break
    sleep 0.1
  done
  [[ $(cat "$work/launched-args") == "$uri" ]] || {
    echo "the desktop launcher did not forward the complete URI" >&2
    exit 1
  }
fi
if command -v xdg-open >/dev/null 2>&1 && command -v xdg-mime >/dev/null 2>&1; then
  mkdir -p "$work/protocol-config"
  XDG_CONFIG_HOME="$work/protocol-config" XDG_DATA_HOME="$work/standard/share" \
    xdg-mime default zapfast.desktop x-scheme-handler/whatsapp
  # xdg-open only tries desktop MIME handlers when a display is declared.
  # The fake executable needs no display server, including on headless CI.
  DISPLAY=:0 ZAPFAST_TEST_ARGS="$work/xdg-args" XDG_CURRENT_DESKTOP=Hyprland \
    XDG_CONFIG_HOME="$work/protocol-config" XDG_DATA_HOME="$work/standard/share" \
    xdg-open "$uri"
  [[ $(cat "$work/xdg-args") == "$uri" ]] || {
    echo "xdg-open did not forward the complete URI to the installed binary" >&2
    exit 1
  }
fi
check_prefix "$work/with%percent"
# A backslash followed by `n`: `awk -v` would turn this into a newline, so the
# Exec would name a path the binary was never installed to.
check_prefix "$work/with\\nbackslash"

# An explicit prefix wins over XDG_DATA_HOME, so the test never writes into
# the developer's real data directory.
XDG_DATA_HOME="$work/elsewhere" check_prefix "$work/explicit" plain
test ! -e "$work/elsewhere" || { echo "wrote into XDG_DATA_HOME despite a prefix" >&2; exit 1; }

# A relative prefix becomes absolute, since a launcher cannot resolve it.
(cd "$work" && bash "$script_dir/install-user.sh" "$fake" relative >/dev/null)
grep -qxF "Exec=$work/relative/bin/zapfast %u" "$work/relative/share/applications/zapfast.desktop" || {
  echo "a relative prefix left a relative Exec" >&2
  exit 1
}

echo "install-user.sh wrote a valid, escaped launcher entry for every prefix."
