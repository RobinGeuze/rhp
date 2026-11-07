# Framework fixture apps

Real, pinned framework applications that rhp is measured against: a
compile-only ratchet (`rhp-difftest lint`), the request diff
(`rhp-difftest app`: `rhp --cgi` against php-cgi) and the console battery
(`apps.py console`: `bin/console`, `artisan` and wp-cli under php and rhp).
The pinned dev tools (PHPUnit, PHPStan, Psalm, PHP-CS-Fixer, Rector) are
measured on their own fixture project by `tools.py`; see "Dev tools" below.

| app           | framework                                   | notes                                   |
|---------------|---------------------------------------------|-----------------------------------------|
| `symfony-7.4` | `symfony/skeleton` 7.4 + `symfony/twig-bundle`, all `symfony/*` at `7.4.*` | `APP_ENV=prod`, attribute routes in `src/Controller/HelloController.php` |
| `symfony-8.1` | `symfony/skeleton` 8.1 + `symfony/twig-bundle`, all `symfony/*` at `8.1.*` | the `symfony-7.4` fixture (same `.env`, controller, templates and corpus) on Symfony 8 |
| `laravel-13`  | `laravel/laravel` 13, every `symfony/*` component pinned to `7.4.*` | `APP_ENV=production`, `APP_DEBUG=false`, file session/cache, sync queue, no database |
| `wordpress-7.1` | WordPress 7.1.4 (downloaded, not committed) + SQLite Database Integration 3.1.3 | SQLite database seeded by `setup.php`, pretty permalinks, cron and updates off; console: wp-cli 2.12.0 |

`apps.toml` is the manifest: one `[[app]]` entry per app with its docroot,
front controller, console entry, pinned downloads, install/setup commands,
the paths to lint and the smoke requests. `apps.py` drives everything from
it, and CI builds its job matrix from it, so the workflow never needs
editing for a new app.

## What is committed

`composer.json`, `composer.lock`, app code, config, templates and a committed
`.env` with fixed, **test-only** secrets (`APP_SECRET`, `APP_KEY`). These are
not secrets: they make cookies, CSRF tokens and signed values reproducible
between php and rhp. Never committed: `vendor/`, `var/`, `storage/` contents,
`bootstrap/cache/*.php`, `node_modules/`, `public/build/` (see `.gitignore`).

## Origin and licence

Most committed app files come from upstream skeletons, which are under the
MIT licence (its text is in the repository's LICENSE-MIT; only the copyright
line differs):

- `laravel-13`: the `laravel/laravel` 13 application skeleton (app code,
  config, routes, views such as `welcome.blade.php`), MIT as its
  `composer.json` and `README.md` state.
- `symfony-7.4` and `symfony-8.1`: `symfony/skeleton` 7.4 and 8.1 plus the
  files their Symfony Flex recipes add (`bin/console`, `config/`,
  `public/index.php`, ...), MIT, "Copyright (c) Fabien Potencier". Their
  `composer.json` says `"MIT"` like upstream's (Flex's create-project had set
  `"proprietary"`). `symfony-8.1` leaves out the skeleton's `AGENTS.md` and
  `.env.dev`, as `symfony-7.4` has no `.env.dev`.
- `wordpress-7.1`: no WordPress code is committed. WordPress itself and the
  SQLite Database Integration plugin are GPL-2.0-or-later, wp-cli is MIT;
  `apps.py setup` downloads all three (see "WordPress" below) into
  git-ignored paths. `wp-config.php` and `setup.php` are rhp's own, as
  THIRD-PARTY-NOTICES says.

The fixture routes, controllers and templates, the request corpora and the
dev-tool fixture project (`tools/project`) are rhp's own.

## Running

```sh
python3 tests/apps/apps.py setup            # downloads, composer install --no-scripts, setup commands
python3 tests/apps/apps.py smoke            # console --version + routes under `php -S`
cargo build --release -p rhp-difftest
python3 tests/apps/apps.py lint             # rhp-difftest lint --check-known, per app
python3 tests/apps/apps.py lint laravel-13 --write-known   # regenerate one list
```

`composer install` always runs with `--no-scripts`: Laravel's post-install
scripts run `key:generate` and `migrate` (which needs sqlite), and Symfony's
run `cache:clear`. Whatever an app actually needs (`package:discover`,
`cache:warmup`) is listed under `setup` in the manifest and run with php.

## WordPress

`wordpress-7.1` has no `composer.json`. Its manifest entry pins three
downloads (`[[app.download]]`: URL, sha256, destination), which `apps.py
setup` fetches into `$RHP_APPS_CACHE` (default `~/.cache/rhp-apps`), checks
and unpacks:

- WordPress 7.1.4 from wordpress.org into `wordpress/` (the docroot). The
  sha256 was taken from the tarball whose sha1 matched wordpress.org's
  published one; the manifest records both.
- [SQLite Database Integration](https://wordpress.org/plugins/sqlite-database-integration/)
  3.1.3 into `wordpress/wp-content/plugins/`. wordpress.org publishes no
  checksum for plugin zips, so its sha256 is the one taken when it was
  pinned.
- wp-cli 2.12.0 (`wp-cli.phar`, the console; `wp-cli.yml` points it at
  `wordpress/`), checked against the sha256 published with the release.

Then `php setup.php` copies the plugin's `db.copy` to
`wordpress/wp-content/db.php` (the drop-in that makes `$wpdb` talk to
SQLite through PDO), deletes `var/database/` and installs with WordPress's
own `wp_install()`: title "rhp fixture", user `admin` / `rhp-fixture-password`
(`admin@example.com`, no mail sent), pretty permalinks `/%postname%/` with
the rewrite rules stored, the plugin activated, and every post, comment and
user dated `2026-01-01 12:00:00`. The default content is the corpus: the
"Hello world!" post, the "Sample Page" page and the comment.

`wp-config.php` sits one directory above `wordpress/`, where WordPress
looks for it. It keeps the database in `var/database/wordpress.sqlite`
(outside the docroot) in SQLite's DELETE journal mode rather than the
plugin's default WAL (a process that exits without closing the database
leaves `-wal`/`-shm` files, which the next request deletes, as setup did on
Linux), fixes the keys and salts (test-only), turns off
WP-Cron, updates and outgoing HTTP, and sets `WP_HOME`/`WP_SITEURL` to
`http://<HTTP_HOST>`, so the site never redirects to a canonical host
(`localhost` in the request diff, `127.0.0.1:<port>` under `smoke`).

Two harness rules exist for it. In the request diff, a path that names a
`.php` file under the docroot (`/wp-login.php`) runs that file, as a web
server's `location ~ \.php$` does; every other path runs the front
controller. `router = false` makes `smoke` start `php -S` without a router
script, which behaves the same. The corpus's `state` is `var/database`. The
first request after setup writes to it (the cron schedule with its
timestamps, the default widgets), so the corpus uses `warmup = "corpus"`
like Symfony's: one php-cgi pass, then both engines start from that
database, and a request that only reads leaves it as it was (the self-check
shows 0 files written). WordPress's nonces change
every 12 hours, so the one on the login page is a `token`.

The plugin needs `pdo_sqlite`. CI's setup-php installs `pdo` and
`pdo_sqlite` with its default extensions and its `-n` shims load them, as
php-cgi runs with `-n` in the request diff (Homebrew's php has both built
in); rhp has both built in.

## The lint ratchet

`rhp-difftest lint <dir>...` parses and compiles every `.php` file the way
rhp compiles an included file, and lists the files it refuses (parse error,
unsupported construct, compile error, compiler panic) with the reason and a
summary by reason. Each app's `known-refused.txt` lists the files rhp refuses
today (keys are `<scanned dir name>/<path>`, e.g. `vendor/symfony/...`).

- CI fails only on a refused file that is **not** on the list.
- Listed files that now compile are reported (a `::notice` in CI) and never
  fail the job; delete those lines, or regenerate the list with
  `--write-known`, in the change that fixed them.
- `--php-lint` additionally runs `php -l` on each refused file and sets aside
  the ones php rejects too (broken or deliberately invalid vendor files).

## The request diff (`rhp-difftest app`)

```sh
cargo build --release -p rhp-cli -p rhp-difftest
python3 tests/apps/apps.py diff                       # --check-known, per app
target/release/rhp-difftest app laravel-13 --out /tmp/appdiff -v
target/release/rhp-difftest app symfony-7.4 --candidate "$(command -v php-cgi)"  # self-check
```

Each app's `requests.toml` is its request corpus: `[[request]]` entries with
an `id`, `method`, `path` (plus query), `headers` (`"Name: value"` strings),
`body`, `content_type`, an optional `flow` and the expected `status`.
Requests in one flow share a cookie jar; other requests start with an empty
one. The `status` is checked against the reference only (any status below
500 when it is left out): if php-cgi doesn't answer as the corpus says, the
run stops with exit code 2, because a broken php-cgi setup would otherwise
make every request `DIFF`, or `SAME` when both engines print the same error
page. Header values and bodies
can use `{cookie:NAME}`, which is the jar's URL-decoded value, and
`{cookie-plain:NAME}`, which is the decrypted Laravel cookie value.

### How a run works

1. The harness snapshots the app's writable `state` dirs, and produces the
   pristine state (see warm-up below).
2. It runs the whole corpus through the reference
   `php-cgi -n -d opcache.enable=0`, then through the candidate
   `rhp --cgi -n -d opcache.enable=0`. The runs are sequential and use the
   same absolute app path. The state is restored from the pristine snapshot
   before each engine. Every request gets `env -i` plus `PATH`, `HOME`,
   `TZ=UTC`, `LANG=C` and the CGI variables (`SCRIPT_FILENAME`,
   `SCRIPT_NAME`, `REQUEST_URI`, `QUERY_STRING`, `REQUEST_METHOD`,
   `HTTP_HOST`, `SERVER_*`, `REMOTE_ADDR`, `CONTENT_TYPE`/`LENGTH`,
   `HTTP_COOKIE`, `HTTP_*` from `headers`, `REDIRECT_STATUS=1` and
   `DOCUMENT_ROOT`). The body goes on stdin. Each engine keeps its own cookie
   jar: php's cookies are never replayed to rhp.
3. Responses are parsed into a status, an ordered list of headers, the body
   and stderr. Named rules then normalize them, and the report lists the
   rules that fired:

   | rule | what it does |
   |------|--------------|
   | `date-header` | Rewrites `Date`, `Last-Modified` and `Expires` values to `<HTTPDATE>` |
   | `cookie-expires` | Rewrites the `expires=` attribute of a `Set-Cookie` to `<DATE>`. `Max-Age` is kept |
   | `php-version` | Rewrites php's version in `X-Powered-By` to the candidate's |
   | `laravel-cookie` | Decrypts encrypted cookies with the `APP_KEY` from `.env` (AES-256-CBC, with the MAC checked) and shows them as `decrypted(<prefix>\|<value>)` |
   | `token-rename` | Renames each capture of the corpus `tokens` patterns, and each decrypted cookie value, to `<TOKn>`. Numbering follows the order of first appearance in the run, so equal tokens stay equal across headers, bodies, steps and written files |
   | `log-timestamp` | Masks ISO 8601 and `Y-m-d H:i:s` timestamps in stderr and in written files |
   | `mask:<name>` | Applies a corpus `[[mask]]` (`name`, `pattern`, `replace`, `in = ["body", "headers", "stderr", "files"]`) |

4. The files each engine created, changed or deleted under the state dirs,
   such as compiled Blade views, the Twig cache and session files, are
   normalized the same way and compared as the pseudo-request `@files`.

When the run ends, the state dirs are put back as they were found.
`--check-known known-passing-requests.txt` fails only when a listed id is not
`SAME`. Ids that are newly `SAME` are printed as hints. Both lists start
empty, because rhp can't serve the apps yet. `--write-known` regenerates a
list. Run it twice to check determinism.

### Warm-up: the Symfony prod cache

A cache warmed by `bin/console cache:warmup` is built by the CLI SAPI: the
container is compiled under `PHP_SAPI=cli`, and the cache adapter is chosen
by `PhpFilesAdapter::isSupported()`, which looks at the SAPI and the opcache
state. So the Symfony corpus sets `warmup = "corpus"`. The harness first
clears `var/cache` (what setup's CLI warm-up left), then runs the whole
corpus once through php-cgi. That snapshot is the pristine state for both
engines, and is the same thing a deploy does: warm once, serve many.

The snapshot then drops `var/cache/prod/twig` (`warmup_drop`), so each engine
compiles the Twig templates itself and the compiled output is diffed under
`@files`. With the container warm, rhp only has to *serve* the app; whether it
can also *compile* the container is a separate check for later, run from an
empty cache. Laravel uses `warmup = "none"`: its views compile lazily, so
each engine compiles them.

## The console battery (`apps.py console`)

```sh
cargo build --release -p rhp-cli
python3 tests/apps/apps.py console                    # every app, --check-known
python3 tests/apps/apps.py console laravel-13 -- --only inspire,w-queue
python3 tests/apps/apps.py console wordpress-7.1 -- --candidate "$(command -v php)"  # self-check
python3 tests/apps/apps.py console symfony-7.4 --write-known
```

Each app's `console.toml` (the manifest's `console_list`) lists console
commands: `bin/console` for Symfony (33: `about`, `debug:*`, `lint:*`,
`cache:*`, `list` in every format, ...), `artisan` for Laravel (56 commands,
10 `tinker --execute` snippets and 10 workload scripts in
`laravel-13/console/`, each booting the console kernel and printing
support, view, security, validation, mail, HTTP, queue, database and
storage results) and wp-cli for WordPress (73: post, user, option, term,
comment, menu, widget, rewrite, cron, role, export, search-replace, media,
i18n, db, `eval` one-liners and the `eval-file` probes in
`wordpress-7.1/console/`). `console.py` runs them; its docstring has the
file format.

How a run works:

1. `target/apps-console/<app>/run/` gets a copy of the app. The `shared`
   paths (`vendor/`, WordPress core, `wp-cli.phar`) are copied once and
   never reset or compared, so they must stay read-only; the rest is copied
   fresh. A copy rather than symlinks: PHP resolves included paths, so a
   symlinked `vendor/` would autoload the app's classes from tests/apps.
2. `clear` paths are removed and the `prepare` commands run with `php -n`
   (Symfony: `cache:warmup` from an empty `var/`; WordPress: two boots that
   schedule its cron events). The result is the pristine state.
3. Each command runs from the pristine state, first as `php -n`, then as
   `rhp -n` (plus the list's `ini` settings), in `run/`, with `env -i`
   plus `PATH`, `HOME` and `TMPDIR` (emptied before each run), `TZ=UTC`,
   `LANG=LC_ALL=C`, `COLUMNS=120`, `NO_COLOR=1`, `TERM=dumb`, and stdin
   closed. After each run the files it changed are put back.
4. A command is `SAME` when the exit code, stdout, stderr and every file it
   created, changed or deleted are identical after the masks, with their
   permission bits. SQLite databases are compared by their SQL dump, other
   binary files by size and md5.

Masks only hide what php itself varies from run to run (the self-check
against php reports every command `SAME`, three runs in a row), plus the
engines' paths:

| app | masks |
|-----|-------|
| all | the engine binary's path (`<PHP_BINARY>`) |
| Symfony | the container's build time and build id (`cache:clear`), the time `about` prints |
| Laravel | migration file times, `generated::` route names, durations and their dot padding, datetimes and Unix times within 3 days of now, session file names and CSRF tokens, `Inspiring` quotes (`inspire`, `make:component`, `make:view`), the key `key:generate --show` prints |
| WordPress | datetimes and Unix times within 3 days of now, the export's channel date, Site Health's server time, password hashes, wpdb's random `%` placeholder |

Each app's `known-passing-console.txt` lists the commands that are `SAME`
under rhp; CI's `apps-lint` job fails only when a listed one is not.
Commands that newly match are printed as hints; regenerate the list with
`--write-known` (run it twice). Outputs and diffs of the last run are in
`target/apps-console/<app>/out/`.

## Adding a framework version

1. Create the app in `tests/apps/<framework>-<version>/`, e.g.
   `composer create-project symfony/skeleton:"8.1.*" symfony-8.1`, then add
   the same fixture routes as the existing app of that framework (copy its
   controller/routes/templates and committed `.env`).
2. Pin the framework: constrain every `symfony/*` component (or the
   framework's equivalents) to the version in `composer.json`, then
   `composer update --no-scripts`.
3. Add an `[[app]]` entry to `apps.toml` (copy the existing one for the same
   framework and change `name`/`version`).
4. `python3 tests/apps/apps.py setup <name>` and `smoke <name>` must pass under
   php; then `cargo build --release -p rhp-difftest` and
   `python3 tests/apps/apps.py lint <name> --write-known` to create its
   `known-refused.txt`.
5. Copy `requests.toml`, `console.toml` (and `console/`) and empty
   `known-passing-requests.txt` and `known-passing-console.txt` from the
   existing app of that framework. `python3 tests/apps/apps.py diff <name> --
   --candidate "$(command -v php-cgi)"` must report every request `SAME`,
   and `python3 tests/apps/apps.py console <name> -- --candidate "$(command
   -v php)"` every command.
6. Commit the app files, the lockfile, the lists and the manifest entry. The CI
   matrix picks the new entry up automatically.

## Dev tools (`tools.py`)

PHPUnit, PHPStan, Psalm, PHP-CS-Fixer and Rector, run on a small fixture
project under php and under rhp.

```sh
cargo build --release -p rhp-cli
python3 tests/apps/tools.py setup                  # composer install in tools/project
python3 tests/apps/tools.py diff                   # every tool; or name some: diff phpstan psalm
python3 tests/apps/tools.py diff rector --only 5,6 # selected command numbers
python3 tests/apps/tools.py diff phpunit --candidate "$(command -v php)"   # self-check
```

- `tools/project/` is the fixture: four classes in `src/` with deliberate
  issues (old array syntax, `strpos() !== false`, missing types, an unused
  variable, a call to a missing method, a wrong `@return`, a possibly-null
  call, badly formatted code), PHPUnit tests in `tests/` (12 tests: data
  provider, expected exception, one skipped and one failing), and each tool's
  configuration (`phpunit.xml`, `phpstan.neon`, `psalm.xml`,
  `.php-cs-fixer.dist.php`, `rector.php`, plus `phpstan-parallel.neon`,
  `.php-cs-fixer.parallel.php` and `rector-parallel.php` for runs with 2
  parallel workers). All caches are configured into `var/`.
- The tools are installed together: `composer.json` and `composer.lock` are
  committed, and the lock pins each version. `vendor/` and `tools/work/` are
  never committed.
- `tools.toml` lists the commands per tool: `--version`, `list`/`help`, the
  main analyse/test/fix commands with common flags, each output format and a
  parallel run where the tool has one.
- Each command runs on a fresh copy of the fixture in `tools/work/<tool>/`
  (a clone of `vendor/` stays there), first under php, then under rhp, with
  a fixed environment: `HOME` and `TMPDIR` inside the run directory, `TZ=UTC`,
  `LANG=LC_ALL=C`, `COLUMNS=120`, `NO_COLOR=1`, `TERM=dumb`, no CI variables,
  stdin closed. A command is `SAME` when the exit code, stdout and stderr are
  byte-identical after normalisation and the files it left in the project
  (outside `vendor/` and `var/`) are identical too. Outputs and diffs are kept
  in `tools/work/<tool>-out/`.

Normalisation only masks what php itself varies from run to run (the
self-check against php must report every command `SAME`), plus the engine's
path:

| rule | what it masks |
|------|---------------|
| `binary` | The engine binary's path (`PHP_BINARY`), which differs by construction |
| `phpunit-time` | PHPUnit's `Time: 00:00.047, Memory: 18.00 MB` line |
| `phpunit-event-time`, `phpunit-event-memory` | The timestamps and memory figures in `--log-events-text` |
| `report-time`, `xml-time`, `xml-memory`, `json-time`, `json-memory` | Durations and memory figures in JUnit, TeamCity, XML and JSON reports |
| `teamcity-flow` | TeamCity's `flowId`, the process id |
| `progress-steps` | The intermediate steps of a Symfony progress bar (it redraws at most every 0.1 s); the first and last step are kept |
| `elapsed`, `memory` | Any other `<number> s/ms/seconds` and `<number> MB/MiB/...`, as in "in 0.088 seconds, 18.00 MB memory used" |

## Findings recorded when the fixtures were created

- **Property hooks:** Symfony 7.4.22's and Laravel 13.39's vendor trees
  contain no property hooks or asymmetric visibility (grepped for property
  declarations followed by `{`; the only `private(set)` text is code that
  Symfony's `var-exporter` *generates* for user classes that have them).
  Symfony 8.1 uses hooks on `Request`/`Response` (every request), which is why
  the first fixtures pinned 7.4; `symfony-8.1` measures 8.1.11 next to it
  (all of its files compile).
- **Laravel and Symfony 7.4:** `laravel/framework` 13 requires
  `^7.4.0 || ^8.0.0` for every Symfony component it uses, so the 7.4 pins
  resolve without conflicts.
- **No database on the request path:** a `GET /` under php-cgi includes ~500
  files; `DatabaseServiceProvider` boots and constructs `DatabaseManager` and
  `ConnectionFactory`, but no connection, connector or PDO class is loaded,
  and no `database/database.sqlite` exists.
