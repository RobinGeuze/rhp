# The request layer (`rhp-sapi`)

rhp runs PHP the way php's SAPI layer does: the engine never talks to a
terminal, a socket or a web server directly. Something outside it — the
`rhp` command, a CGI front end, later a web server that embeds rhp — hands
it one request and receives the response through a small trait. This
document describes that boundary, the request lifecycle behind it and the
threading model, and where it is headed (`rhp -S`, embedding in a server,
FastCGI).

php-src counterparts: `main/SAPI.c` (request info, headers),
`main/php_variables.c` (superglobals), `main/rfc1867.c` (uploads),
`main/output.c` (output buffering), `ext/standard/head.c` (header
builtins) and `sapi/cgi/cgi_main.c` (the CGI front end).

## Layers

```
 server / front end        rhp-cli (argv)   rhp --cgi        future: -S, embed, FastCGI
          │                      │               │                     │
          ▼                      ▼               ▼                     ▼
 rhp-sapi            run_cli()          cgi::main()            run_request()
                         └──────────────┬───────┴──────────────────┘
                                        ▼
                     run_request(Box<dyn Sapi>, Request) -> Outcome
                                        │   one fresh Vm per call
                                        ▼
 rhp-vm              Vm: set_sapi → set_ini → set_request → start_request
                         → run_main → finish_request
                                        │
 rhp-builtins        sapi::{Sapi, Response, RequestInfo, header_op},
                     query_fns (treat_data), rfc1867 (multipart), output layer
```

- **`rhp-builtins::sapi`** defines the contract: the `Sapi` trait, the
  `RequestInfo` a server fills in, and the `Response` header store, which
  `header()`, `setcookie()`, `http_response_code()` and the rest drive the
  way `sapi_header_op` does.
- **`rhp-vm::request`** turns a `RequestInfo` into `$_SERVER`, `$_GET`,
  `$_POST`, `$_COOKIE`, `$_FILES`, `$_ENV` and `$_REQUEST`, reads POST
  bodies (`post_max_size`, `enable_post_data_reading`, multipart), and sends
  the headers lazily.
- **`rhp-sapi`** is the library entry point (`run_request`) plus the two
  shipped SAPIs, `CliSapi` (php-cli) and `cgi::CgiSapi` (php-cgi, with
  `cgi::main` porting `cgi_main.c`'s argv and environment handling).
- **`rhp-cli`** is a thin argv front end: `rhp script.php`, `rhp -r code`,
  `rhp --cgi …`.

## The embedding API

```rust
use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Response, Sapi, Script};

struct MySapi { /* the connection, a response builder, … (may hold Rc) */ }

impl Sapi for MySapi {
    fn name(&self) -> &str { "my-server" }                     // php_sapi_name()
    fn ub_write(&mut self, body: &[u8]) { /* send body bytes */ }
    fn send_headers(&mut self, r: &Response) { /* r.code, r.status_line, r.headers */ }
    fn flush(&mut self) { /* push buffered bytes to the client */ }
    fn log_message(&mut self, line: &[u8]) { /* error_log / log_errors */ }
}

let outcome = run_request(
    Box::new(MySapi { /* … */ }),
    Request {
        script: Script::File("/srv/app/public/index.php".into()),
        ini: vec![("display_errors".into(), "0".into())],
        args: Vec::new(),
        info: Some(RequestInfo {
            server: vec![/* (b"REQUEST_URI", b"/x?y=1"), (b"HTTP_HOST", …), … */],
            method: Some(b"GET".to_vec()),
            query_string: Some(b"y=1".to_vec()),
            cookie: None,
            response_code: 200,
            ..Default::default()
        }),
        php_binary: None,
    },
);
```

### `Sapi` (php's `sapi_module_struct`)

| method | php | default |
|---|---|---|
| `name` | `sapi_module.name` → `php_sapi_name()`, `PHP_SAPI` | required |
| `ini_defaults` | `sapi_module.ini_defaults` (below `-d` and `ini_set`) | none |
| `stores_headers` | `header_handler`: false keeps `headers_list()` empty (php-cli) | `true` |
| `initial_response_code` | status before any `header()`: 0 for cli, 200 for web | `200` |
| `ub_write` | unbuffered body writer, called with what leaves the bottom of the output-buffer stack | required |
| `flush` | `flush()`, `implicit_flush`, request end | no-op |
| `send_headers` | called **once**: right before the first body byte, or at request end when there is no body. Not called for `no_headers` (`php-cgi -q`). The `Response` holds the final status code, an explicit status line if `header("HTTP/1.1 …")` set one, and the header lines in order (default `Content-type` and `X-Powered-By` included) | no-op |
| `log_message` | one `error_log` / `log_errors` line, no newline | stderr |
| `write_stderr` | `display_errors=stderr` output | stderr |

The SAPI decides the wire format. `CgiSapi` writes `Status:` (when not
200) + header lines + a blank line + the body to stdout; an HTTP server
writes a status line and headers to its connection; a FrankenPHP-style
embedding hands them to the host server's response writer.

### `RequestInfo` (php's `SG(request_info)` + server variables)

Byte strings throughout (PHP is binary-safe). Everything is optional:
`RequestInfo::default()` is a request with no input.

| field | becomes |
|---|---|
| `server` | `$_SERVER`, in order (what `register_server_variables` adds: for CGI, the environment then `PHP_SELF`). The engine appends `PHP_AUTH_*`, `REQUEST_TIME_FLOAT`, `REQUEST_TIME`, `argv`/`argc` |
| `env` | `$_ENV`, `getenv()` |
| `method` | `REQUEST_METHOD`: POST parsing, HEAD (body discarded), the Location → 302/303 rule |
| `query_string` | `$_GET` (`arg_separator.input`, `max_input_vars`, `max_input_nesting_level`) |
| `cookie` | `$_COOKIE` |
| `content_type`, `content_length`, `body` | `$_POST`, `$_FILES`, `php://input`, `request_parse_body()`. The body must be fully read before the call (see *Gaps*) |
| `authorization` | the `Authorization` header: `PHP_AUTH_USER` / `PHP_AUTH_PW` (Basic) or `PHP_AUTH_DIGEST` |
| `headers` | `getallheaders()` / `apache_request_headers()` |
| `request_time` | `REQUEST_TIME_FLOAT` (seconds since the epoch) |
| `argv` | `$argv` / `$_SERVER['argv']` for a SAPI that has one (php-cgi run from a shell) |
| `no_headers` | never send headers (`php-cgi -q`) |
| `response_code` | the initial status (php-cgi: 200, or 404 when no script was found) |
| `proto_num` | `SG(request_info).proto_num` (1000 = HTTP/1.0, 1001 = HTTP/1.1): the Location → 303 rule |

`info: None` means a CLI run: superglobals come from the process
environment and `$argv` from `Request::args`.

### `Outcome`

`Exit(code)` (the script's exit status; 255 after a fatal error or a parse
error) or `CannotRead { path, error }` when the script file can't be read.
The HTTP status is in the `Response` that `send_headers` received, not in
`Outcome`.

### Lower level

`run_request` is a convenience over `Vm`. A server that needs to step
in between can drive it directly:
`Vm::new()`, `set_sapi` (right away, before anything else), `set_ini` for
each setting, `set_request`, `start_request` (post-size check,
`X-Powered-By`), compile, `run_main` (script, shutdown functions,
destructors, output-buffer flush, headers if still unsent), and finally
`finish_request` (headers, `Sapi::flush`, upload cleanup). Dropping the
`Vm` also deletes this request's uploaded temp files. Compile the main
script with `Vm::compile_main` to go through the compiled-script cache
(below); run `opcache.preload` first with `Vm::preload_pending` and
`Vm::run_preload` on an engine of its own, as `run_request` does.

## The compiled-script cache (opcache)

`rhp-vm`'s `opcache` module is Zend OPcache without the optimizer and the
JIT. A request's main script and every `include` / `require` of an
unchanged file reuse its compiled `Program`, with php's settings:
`opcache.enable` (web SAPIs; on by default) / `opcache.enable_cli`,
`validate_timestamps`, `revalidate_freq`, `file_update_protection`
(a file changed in the last 2 s is compiled but not cached),
`max_accelerated_files`, `max_file_size`, `memory_consumption` and
`blacklist_filename`. `opcache_get_status()` and the other `opcache_*`
functions report it; `opcache_reset()` takes effect at the next request.

- **Per thread.** A `Program` holds `Rc`s, so the cache is a
  `thread_local!`: every request served on a worker thread shares it, as
  php-fpm workers share their shared-memory segment. Once bytecode is
  `Arc`-based it can become process-wide behind a lock; only the module
  touches it.
- **Nothing per-request is cached.** A `Program` is immutable and the VM
  copies the protos it runs into the request, so statics, class statics,
  constants and globals start fresh in every request.
- **Preload.** `opcache.preload` runs once per thread, before its first
  request, on an engine of its own (its output goes to stdout). The
  functions and classes it declared are declared again at the start of
  every request; a preloaded file included later doesn't redeclare them.
  A failed preload (parse error, uncaught exception) fails that request
  and every later one on the thread with exit status 1, as php refuses
  to start (`Vm::preload_outcome` tells an embedder that drives `Vm`
  itself).
- **Not implemented:** the file cache (`opcache.file_cache`: compiled
  units aren't serialisable), the optimizer and its debug output, the JIT
  (`opcache_get_status()['jit']` reports what the settings select),
  switching to `opcache.preload_user` (only its warning), the inheritance
  cache, `opcache.enable_file_override`, and caching `phar://` scripts.

## Request lifecycle

What happens inside one `run_request`, in php's order:

1. **Engine setup.** A fresh `Vm`. `set_sapi` installs the SAPI name and ini
   defaults for this thread and defines `PHP_SAPI`. `-d`-style settings
   apply over them.
2. **`sapi_activate`.** `set_request` records the method, protocol and HEAD
   flag. `start_request` checks the POST body against `post_max_size`
   ("PHP Request Startup: POST Content-Length …", in Unknown on line 0)
   and adds `X-Powered-By` when `expose_php` is on.
3. **`php_request_startup`.** The output layer starts first (so input-var
   warnings land in an `output_buffering` buffer), then the superglobals
   are built per `variables_order` / `request_order`. Multipart bodies go
   through the `rfc1867` parser into `$_POST` / `$_FILES`, with temp files
   in `upload_tmp_dir`.
4. **Compile and run.** A parse or compile error is reported per
   `display_errors` / `log_errors` (`html_errors` is on for web SAPIs) with
   exit status 255.
5. **Output.** `echo` goes through the output-buffer stack. Whatever leaves
   the bottom goes to `Sapi::ub_write`, and the first byte triggers
   `send_headers` (after `header_register_callback`'s callback runs). A
   `header()` after that is the usual "headers already sent" warning.
6. **Shutdown.** Shutdown functions, destructors, output-buffer flush, then
   headers if nothing was output (a fatal error turns a 200 into a 500, as
   php does), `Sapi::flush`, and deletion of uploads not moved by
   `move_uploaded_file()`.

## Threading model

**One `Vm` per OS thread, values stay plain `Rc`.** Engine values (`Value`,
arrays, objects, the `Sapi` box) are deliberately neither `Send` nor
`Sync`: reference counting stays cheap and non-atomic, as in php's NTS
build. Concurrency comes from threads, never from sharing an engine:

- A server keeps a pool of long-lived worker threads and runs
  `run_request` on one of them per request. The `Sapi` is built *on* that
  thread, so it may hold `Rc`s; it talks to the network side through
  channels or other `Send` handles.
- Every request gets a **fresh `Vm`**: nothing a script defines (globals,
  functions, classes, constants, statics, ini changes, output buffers,
  headers) survives into the next request. This matches php-fpm and
  FrankenPHP's classic mode. Worker mode (one bootstrapped app serving many
  requests) can come later as a separate API.
- The per-thread state outside the `Vm` (SAPI name and ini defaults,
  request-scoped builtin state) is in `thread_local!`s and is reset for
  each request. `rhp-sapi`'s `concurrent_requests_on_worker_threads_are_isolated`
  test runs requests on several threads at once and checks that none of
  this leaks across threads or between requests on one thread.
- **Process-wide state.** Like php's ZTS builds, some things belong to the
  OS process, not the thread: the working directory (`chdir()`, and
  `cgi::main`'s chdir to the script's directory), environment changes from
  `putenv()`, `umask()`, signal handlers (`pcntl`), and the shared caches
  (builtin registry, TLS and HTTP connection pools in `curl`), which are
  read-only or locked. A multi-threaded server must not rely on chdir; a
  per-`Vm` working directory is future work (see *Gaps*).

## Where this goes

- **`rhp -S` (built-in web server).** An HTTP/1.1 listener in `rhp-cli` (or
  a small `rhp-server` crate) that maps a request to a `RequestInfo`
  (router script, document root, `PATH_INFO` as `php -S` sets it) and runs
  it on a worker thread through a `Sapi` that writes to the socket. Its
  harness diffs against `php -S`.
- **Embedding in a server (FrankenPHP-like).** The production target:
  Caddy (through a C ABI shim over `run_request`) or a Rust server (hyper /
  axum, calling `run_request` directly from a worker pool). The pieces
  needed beyond today's API: a streaming body reader (`Sapi::read_body`,
  so uploads don't have to be buffered first), `max_execution_time`
  enforcement with an interrupt flag the server can also set when a client
  disconnects (`connection_aborted()`) and a per-`Vm` working directory.
- **FastCGI.** A `cgi-fcgi` front end that reads FastCGI records, builds a
  `RequestInfo` from the params, and writes stdout records through a
  `Sapi`. It reuses `cgi::CgiSapi`'s header logic and `php_sapi_name()`
  stays `"cgi-fcgi"`.

## Gaps

- The request body is read completely before `run_request`; there is no
  streaming reader yet.
- `max_execution_time` / `max_input_time` are not enforced.
- `auto_prepend_file` / `auto_append_file` are ignored (in every SAPI).
- `chdir` is process-wide (see *Threading model*).
- The php-cgi front end cannot read the script from stdin and doesn't
  support `-v -i -m -l -s -w -a`.
- Output layer: no `ob_gzhandler`, no `output_add_rewrite_var`.

## Testing

- `crates/rhp-cli/tests/cgi.rs` runs every `tests/cgi/*.php` (with an
  optional `.cgi` file of `env` / `ini` / `stdin` / `arg` directives)
  under php-cgi and `rhp --cgi` and requires identical stdout, stderr and
  exit status. It is skipped when php-cgi is absent or isn't 8.5.
- `crates/rhp-sapi/tests/` covers the embedding contract (call order,
  isolation, threads) without any php binary.
- `rhp-difftest phpt` runs php-src's `--GET--` / `--POST--` / `--CGI--`
  tests under php-cgi vs `rhp --cgi -C`; the ones that match are in
  `tests/phpt/known-passing.txt`.
- `rhp-difftest app` (via `tests/apps/apps.py diff`) replays a request
  corpus against the pinned Symfony and Laravel fixture apps under both
  engines.
