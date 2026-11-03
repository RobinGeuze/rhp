#!/usr/bin/env bash
# build-from-source.sh <pcre2|libxml2> <version>
#
# Builds the release php links when the distro has no -dev package of it:
# the upstream tarball, checked against sha256sums, installed static-only
# into ~/.cache/rhp-native/<name>-<version>.
set -euo pipefail

name="$1"
v="$2"
prefix="$HOME/.cache/rhp-native/$name-$v"
case "$name" in
  pcre2)
    tarball="pcre2-$v.tar.bz2"
    url="https://github.com/PCRE2Project/pcre2/releases/download/pcre2-$v/$tarball"
    flags=(--enable-jit)
    ;;
  libxml2)
    tarball="libxml2-$v.tar.xz"
    url="https://download.gnome.org/sources/libxml2/${v%.*}/$tarball"
    # What changes behaviour in Debian/Ubuntu's build: zlib on, ICU off.
    flags=(--with-zlib --without-icu --without-python)
    ;;
  *)
    echo "::error::build-from-source.sh: unknown library $name"
    exit 1
    ;;
esac

sum="$(awk -v f="$tarball" '$2 == f { print $1 }' "$(dirname "$0")/sha256sums")"
if [ -z "$sum" ]; then
  echo "::error::php links $name $v, and .github/actions/native-deps/sha256sums has no checksum for $tarball. Add the release tarball's SHA-256 (from the project's release page)."
  exit 1
fi
tmp="$(mktemp -d)"
curl -fsSL --retry 3 -o "$tmp/$tarball" "$url"
echo "$sum  $tmp/$tarball" | sha256sum -c -
tar -xf "$tmp/$tarball" -C "$tmp"
cd "$tmp/$name-$v"
./configure --prefix="$prefix" --disable-shared --enable-static --with-pic "${flags[@]}"
make -j"$(nproc)"
make install
