#!/usr/bin/env bash
# Build php from a php-src checkout with AddressSanitizer + UndefinedBehavior-
# Sanitizer, for the phpt nightly's php bug hunt (rhp-difftest phpt
# --php-findings). Compiler flags mirror php-src's own ASAN CI job. The build is
# non-debug, so php's leak reporter doesn't write into the output being
# compared. Extensions are compiled in (static), so `php -n` keeps them: no shim
# needed.
#
# usage: build-php-sanitizers.sh <php-src dir> <install prefix>
# needs: clang, libclang-rt-dev, autoconf, bison, re2c, pkg-config, make, libxml2-dev,
#        libgmp-dev, zlib1g-dev
set -euo pipefail

src=$1
prefix=$2
# object-size: php-src's ASan CI builds at -O0, where this check is inert. At
# -O1 it fires on every `new X(...)` of a class without a constructor (php hands
# out the static zend_pass_function as a zend_function*): harmless noise.
san="-fsanitize=undefined,address -fno-sanitize=function,object-size"

cd "$src"
./buildconf --force
CC=${CC:-clang} CXX=${CXX:-clang++} \
  CFLAGS="$san -fno-omit-frame-pointer -DZEND_TRACK_ARENA_ALLOC -O1 -g" \
  LDFLAGS="$san" \
  ./configure --prefix="$prefix" \
  --disable-all --enable-cli --disable-cgi --disable-phpdbg \
  --enable-bcmath --enable-calendar --enable-ctype --enable-dom --enable-exif \
  --enable-fileinfo --enable-filter --enable-mbstring --disable-mbregex \
  --with-gmp --enable-pcntl --enable-phar --enable-posix --enable-session \
  --enable-shmop --enable-simplexml --enable-sockets --enable-sysvmsg \
  --enable-sysvsem --enable-sysvshm --enable-tokenizer --enable-xml \
  --enable-xmlreader --enable-xmlwriter --with-libxml --with-iconv --with-zlib
make -j"$(nproc)"
make install
