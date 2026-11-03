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
in between (to cache compiled scripts, for instance) can drive it directly:
`Vm::new()`, `set_sapi` (right away, before anything else), `set_ini` for
each setting, `set_request`, `start_request` (post-size check,
`X-Powered-By`), compile, `run_main` (script, shutdown functions,
destructors, output-buffer flush, headers if still unsent), and finally
`finish_request` (headers, `Sapi::flush`, upload cleanup). Dropping the
`Vm` also deletes this request's uploaded temp files.

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

## Memory

`memory_limit` and `memory_get_usage()` need the binary to install the
counting allocator, once:

```rust
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;
```

It is the system allocator plus per-thread counters (`rhp_values::mem`).
While a request runs on a thread, they follow php's zend_mm: `size` counts
blocks rounded about as zend_mm rounds them, real usage counts 2 MiB
chunks (kept once emptied, as zend_mm caches them) plus huge blocks.
Counting starts when the script starts (`Vm::run`; compiling the main
script is not counted, as with opcache) and stops at `finish_request`.
Without the allocator, requests run, `memory_get_usage()` reads 0, and
only the builtins that check a size up front enforce the limit.

The allocator can't fail or unwind, so it only records that the limit was
crossed. As in php 8.5 (`zend_mm_delayed_error`), the VM raises "Allowed
memory size of N bytes exhausted" at its next interrupt check: a jump,
entering or leaving a function, a builtin returning, the end of the script.
The request then ends like any fatal error: shutdown functions run, the
exit status is 255, a 200 becomes a 500 when the error isn't displayed.
The next request on the thread starts from zero. Builtins that know a big
size first refuse it before allocating, as php's fallible allocations do:
`str_repeat`, `str_pad`, `implode`, `array_fill`, `array_pad`, `range`,
`fread`, `stream_get_line`, `file_get_contents` (plain files) and
`unserialize`.

php's hard limit (twice `memory_limit`, or `memory_hard_limit_percent`)
kills php-cli with exit 137. rhp checks it where the VM or a builtin
allocates a size it knows first but php can't refuse: string concatenation
(`.`, `.=`, interpolation), a far string offset, `hash_pbkdf2`. php-cli
exits as php does; any other SAPI ends just the request with the fatal
error.

What still escapes the limit, until more builtins are mirrored:

- A builtin without a pre-check (`str_replace`, `explode`, `array_merge`,
  …) allocates as much as its input asks for and only reports the
  crossing when it returns. One call can use far more than the limit.
- An allocation the system refuses aborts the process (Rust's
  `handle_alloc_error`), taking every request on it down. php's hard
  limit stops such sizes first; rhp stops them only where it checks.
- Arrays grow without a hard-limit check. Growth is at most a doubling
  per insert, and the crossing is raised at the next interrupt check, so
  a VM loop can't run away; a builtin that builds a big array can.

Limits of the model:

- Per thread. A block freed on another thread than the one that allocated
  it is subtracted from the freeing thread; counters saturate at zero.
  Memory allocated before the request and freed during it (engine
  startup data) makes the counts read low by that much.
- Data kept from one request to the next (a compiled-script cache) belongs
  inside `rhp_values::mem::uncounted`, as php's opcache memory is outside
  the request heap; otherwise the request that built it is charged and a
  later one credited when it is freed.
- rhp's data structures are not php's: an array of ints takes about three
  times php's memory, strings about the same. Scripts close to the limit
  can fail earlier (or later) than under php, and "tried to allocate N" for
  a growing array differs. Sizes a builtin checks up front use php's
  numbers.

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
  disconnects (`connection_aborted()`), a per-`Vm` working directory, and a
  compiled-script cache shared between requests on one thread (a
  thread-local opcache).
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
