#!/bin/sh
# Copy the php-src C files rhp compiles into each crate's vendor/php-src, from
# a php-src checkout at the commit pinned in tests/phpt/php-src.ref: rhp-csrc
# (php-src's permissively licensed C) and rhp-mbfl-sys (libmbfl, LGPL, in its
# own crate). Writes VERSION (tag and commit) and SHA256SUMS next to each copy.
# Running it again on the same checkout changes nothing.
#
#   crates/rhp-csrc/tools/sync-php-src-c.sh <php-src-dir>
set -eu

# Files to vendor into rhp-csrc, grouped by the cargo feature that compiles
# them. Paths are relative to the php-src root and keep their php-src layout.
FILES='
LICENSE
ext/standard/crypt_blowfish.c
ext/standard/crypt_blowfish.h
ext/standard/crypt_freesec.c
ext/standard/crypt_freesec.h
ext/standard/crypt_sha256.c
ext/standard/crypt_sha512.c
'

# rhp-mbfl-sys: every C file and header of libmbfl (found below, so files a
# php release adds come along), its licence and README, and what mbstring.c
# uses next to it: php_unicode.c and its tables, the East Asian width table
# and the rare-codepoint bit vector.
MBFL_FILES='
LICENSE
ext/mbstring/libmbfl/LICENSE
ext/mbstring/libmbfl/README.md
ext/mbstring/php_unicode.c
ext/mbstring/php_unicode.h
ext/mbstring/rare_cp_bitvec.h
ext/mbstring/unicode_data.h
'

die() { echo "sync-php-src-c: $*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: $0 <php-src-dir>"
src=$1
[ -d "$src" ] || die "$src: not a directory"

crate=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$crate/../.." && pwd)

want=$(grep -v '^#' "$root/tests/phpt/php-src.ref" | head -n 1)
have=$(git --no-optional-locks -C "$src" rev-parse HEAD) || die "$src: not a git checkout"
[ "$have" = "$want" ] ||
	die "$src is at $have; tests/phpt/php-src.ref pins $want (git -C $src checkout $want)"
tag=$(git --no-optional-locks -C "$src" describe --tags --exact-match HEAD 2>/dev/null) ||
	die "$src: HEAD $have has no tag"

MBFL_FILES="$MBFL_FILES
$(cd "$src" && find ext/mbstring/libmbfl -type f \( -name '*.c' -o -name '*.h' \) | LC_ALL=C sort)
"

if command -v sha256sum >/dev/null 2>&1; then
	sha256() { sha256sum "$@"; }
else
	sha256() { shasum -a 256 "$@"; }
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# sync <dest> <files>: copy <files> into <dest>, drop what is no longer
# listed, write VERSION and SHA256SUMS.
sync() {
	dest=$1
	files=$2
	# shellcheck disable=SC2086
	git --no-optional-locks -C "$src" diff --quiet HEAD -- $files ||
		die "$src has local changes to vendored files"

	for f in $files; do
		[ -f "$src/$f" ] || die "$src/$f: missing"
		put "$src/$f" "$dest/$f"
	done

	mkdir -p "$dest"
	(cd "$dest" && find . -type f ! -name VERSION ! -name SHA256SUMS | sed 's|^\./||') |
		while read -r f; do
			case "
$files
" in
			*"
$f
"*) ;;
			*) rm "$dest/$f"; echo "removed ${dest#"$root"/}/$f" ;;
			esac
		done
	find "$dest" -type d -empty -delete

	printf '%s\n%s\n' "$tag" "$want" >"$tmp/VERSION"
	put "$tmp/VERSION" "$dest/VERSION"

	# shellcheck disable=SC2086
	(cd "$dest" && sha256 $files) >"$tmp/SHA256SUMS"
	put "$tmp/SHA256SUMS" "$dest/SHA256SUMS"
}

# Replace $2 with $1 only when the content differs, so reruns touch nothing.
put() {
	if ! cmp -s "$1" "$2"; then
		mkdir -p "$(dirname "$2")"
		cp "$1" "$2"
		echo "updated ${2#"$root"/}"
	fi
}

sync "$crate/vendor/php-src" "$FILES"
sync "$root/crates/rhp-mbfl-sys/vendor/php-src" "$MBFL_FILES"
