#!/usr/bin/env bash
# Requires GNU gettext tools with Rust support. Normal Cargo builds do not.
set -euo pipefail
cd "$(dirname "$0")/../.."
mode=${1:-update}
if [[ "$mode" != update && "$mode" != --check ]]; then
    echo "Usage: $0 [--check]" >&2
    exit 2
fi
translation_template=$(mktemp)
trap 'rm -f "$translation_template"' EXIT
xgettext --language=Rust --from-code=UTF-8 \
    --keyword= --keyword=gettext:2 --keyword=ngettext:2,3 --keyword=pgettext:2c,3 \
    --keyword=translated:2 \
    --add-comments=Translators: \
    --flag=ngettext:2:rust-format --flag=ngettext:3:rust-format \
    --package-name=ZapFast --copyright-holder='ZapFast contributors' \
    --msgid-bugs-address='https://github.com/crmne/zapfast/issues/new?template=translation.yml' \
    --files-from=assets/i18n/POTFILES --output="$translation_template"
if [[ "$mode" == --check ]]; then
    # The extraction timestamp is the only nondeterministic header.
    diff -u <(sed '/^"POT-Creation-Date:/d' assets/i18n/zapfast.pot) \
        <(sed '/^"POT-Creation-Date:/d' "$translation_template")
else
    cp "$translation_template" assets/i18n/zapfast.pot
    for catalog in assets/i18n/*.po; do
        msgmerge --update --backup=none "$catalog" assets/i18n/zapfast.pot
    done
fi
for catalog in assets/i18n/*.po; do
    msgfmt --check --check-format --output-file=/dev/null "$catalog"
done
