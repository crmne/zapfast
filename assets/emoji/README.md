# Emoji search keywords

`keywords-en.tsv` holds the English search keywords of each emoji for the
picker: the emoji without its U+FE0F selectors, a tab, and the keywords in
lower case separated by ` | `. It is derived from the annotations of Unicode
CLDR 49 (<https://cldr.unicode.org>) by `scripts/update-emoji-keywords.py`,
which leaves out the names and the skin tone variants. Do not edit it by hand.

CLDR is distributed under the Unicode License, version 3; the full text is in
`THIRD-PARTY-NOTICES.md` at the repository root. The file remains under that
license. The rest of ZapFast is MIT-licensed.
