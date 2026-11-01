# Framework fixture apps

Real, pinned framework applications that rhp is measured against: today a
compile-only ratchet (`rhp-difftest lint`), later the request harness
(`rhp --cgi` / `rhp -S` diffed against php).

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
5. Commit the app files, the lockfile, the list and the manifest entry. The CI
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
