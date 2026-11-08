# A Linux environment close to CI's (ubuntu-latest = Ubuntu 26.04): php and
# php-cgi from the repository setup-php uses on 26.04 (packages.sury.org), the extensions and `-n` shims of
# .github/actions/setup-php, the C libraries of .github/actions/native-deps, and
# stable Rust. Built and used by .github/scripts/linux-ci.sh; see tests/phpt/README.md.
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8

RUN apt-get update -q && apt-get install -y --no-install-recommends \
      ca-certificates curl gnupg tzdata tzdata-legacy \
      build-essential git pkg-config sudo procps iproute2 locales \
 && curl -fsSL -o /usr/share/keyrings/sury-php.gpg https://packages.sury.org/php/apt.gpg \
 && echo "deb [signed-by=/usr/share/keyrings/sury-php.gpg] https://packages.sury.org/php/ resolute main" \
      > /etc/apt/sources.list.d/sury-php.list \
 && apt-get update -q

# setup-php's extension list (.github/actions/setup-php/action.yml). Several are
# part of php8.5-common or compiled in; the rest are their own packages.
RUN apt-get install -y --no-install-recommends \
      php8.5-cli php8.5-cgi php8.5-dev php8.5-common \
      php8.5-bcmath php8.5-bz2 php8.5-curl php8.5-xml php8.5-gmp php8.5-intl \
      php8.5-mbstring php8.5-zip php8.5-sqlite3 php8.5-gd php8.5-readline \
 && php -v && php-cgi8.5 -v

# native-deps: the libraries rhp links (keep in sync with that action).
RUN apt-get install -y --no-install-recommends \
      libpcre2-dev libxml2-dev libicu-dev libssl-dev libjitterentropy3-dev \
      zlib1g-dev libzstd-dev libcurl4-openssl-dev \
      libbrotli-dev libidn2-dev libkrb5-dev libldap-dev libnghttp2-dev \
      libpsl-dev librtmp-dev libssh-dev libssh2-1-dev libzip-dev libgd-dev libgmp-dev \
      libsqlite3-dev libedit-dev

# The releases php links that the distro has no -dev package of (PCRE2, libxml2).
COPY native-deps /opt/native-deps
RUN set -e; ldconfig; \
    have=$(pkg-config --modversion libpcre2-8); want=$(php -n -r 'echo strtok(PCRE_VERSION, " ");'); \
    echo "pcre2: distro $have, php $want"; \
    if [ "$have" != "$want" ]; then HOME=/opt /opt/native-deps/build-from-source.sh pcre2 "$want"; \
      echo "/opt/.cache/rhp-native/pcre2-$want/lib/pkgconfig" >> /opt/pkgconfig-path; fi; \
    have=$(pkg-config --modversion libxml-2.0); \
    want=$(php -n -r '$v = (int) LIBXML_LOADED_VERSION; echo intdiv($v, 10000), ".", intdiv($v, 100) % 100, ".", $v % 100;'); \
    echo "libxml2: distro $have, php $want"; \
    if [ "$have" != "$want" ]; then HOME=/opt /opt/native-deps/build-from-source.sh libxml2 "$want"; \
      echo "/opt/.cache/rhp-native/libxml2-$want/lib/pkgconfig" >> /opt/pkgconfig-path; fi; \
    touch /opt/pkgconfig-path

# A non-root user, as on the runner (root changes file-permission tests).
RUN useradd -m -s /bin/bash runner && echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/runner
USER runner
ENV RUSTUP_HOME=/home/runner/.rustup CARGO_HOME=/home/runner/.cargo
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable -c rustfmt,clippy
# The `-n` shims of .github/actions/setup-php (last: they change most often).
USER root
COPY setup-php/make-shims.sh /opt/make-shims.sh
RUN /opt/make-shims.sh /opt/php-shim \
 && test "$(/opt/php-shim/php -n -r 'echo "ok";')" = ok \
 && /opt/php-shim/php -n -m | tr '\n' ' '
USER runner
ENV PATH=/opt/php-shim:/home/runner/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /src
