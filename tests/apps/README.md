# Framework fixture apps

Real, pinned framework applications that rhp is measured against: today a
compile-only ratchet (`rhp-difftest lint`) and the request diff
(`rhp-difftest app`: `rhp --cgi` against php-cgi).

| app           | framework                                   | notes                                   |
|---------------|---------------------------------------------|-----------------------------------------|
| `symfony-7.4` | `symfony/skeleton` 7.4 + `symfony/twig-bundle`, all `symfony/*` at `7.4.*` | `APP_ENV=prod`, attribute routes in `src/Controller/HelloController.php` |
| `laravel-13`  | `laravel/laravel` 13, every `symfony/*` component pinned to `7.4.*` | `APP_ENV=production`, `APP_DEBUG=false`, file session/cache, sync queue, no database |

`apps.toml` is the manifest: one `[[app]]` entry per app with its docroot,
front controller, console entry, install/setup commands, the paths to lint
and the smoke requests. `apps.py` drives everything from it, and CI builds its
job matrix from it, so the workflow never needs editing for a new app.

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
- `symfony-7.4`: `symfony/skeleton` 7.4 plus the files its Symfony Flex
  recipes add (`bin/console`, `config/`, `public/index.php`, ...), MIT,
  "Copyright (c) Fabien Potencier". Its `composer.json` says `"MIT"` like
  upstream's (Flex's create-project had set `"proprietary"`).

The fixture routes, controllers and templates, and the request corpora, are
rhp's own.

## Running

```sh
python3 tests/apps/apps.py setup            # composer install --no-scripts + setup commands
python3 tests/apps/apps.py smoke            # console --version + routes under `php -S`
cargo build --release -p rhp-difftest
python3 tests/apps/apps.py lint             # rhp-difftest lint --check-known, per app
python3 tests/apps/apps.py lint laravel-13 --write-known   # regenerate one list
```

`composer install` always runs with `--no-scripts`: Laravel's post-install
scripts run `key:generate` and `migrate` (which needs sqlite), and Symfony's
run `cache:clear`. Whatever an app actually needs (`package:discover`,
`cache:warmup`) is listed under `setup` in the manifest and run with php.

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
5. Copy `requests.toml` and an empty `known-passing-requests.txt` from the
   existing app of that framework. `python3 tests/apps/apps.py diff <name> --
   --candidate "$(command -v php-cgi)"` must report every request `SAME`.
6. Commit the app files, the lockfile, the lists and the manifest entry. The CI
   matrix picks the new entry up automatically.

## Findings recorded when the fixtures were created

- **Property hooks:** Symfony 7.4.22's and Laravel 13.39's vendor trees
  contain no property hooks or asymmetric visibility (grepped for property
  declarations followed by `{`; the only `private(set)` text is code that
  Symfony's `var-exporter` *generates* for user classes that have them).
  Symfony 8.1 uses hooks on `Request`/`Response` (every request), which is why
  the fixtures pin 7.4 for now.
- **Laravel and Symfony 7.4:** `laravel/framework` 13 requires
  `^7.4.0 || ^8.0.0` for every Symfony component it uses, so the 7.4 pins
  resolve without conflicts.
- **No database on the request path:** a `GET /` under php-cgi includes ~500
  files; `DatabaseServiceProvider` boots and constructs `DatabaseManager` and
  `ConnectionFactory`, but no connection, connector or PDO class is loaded,
  and no `database/database.sqlite` exists.
