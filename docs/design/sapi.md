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
 rhp-sapi            cli::main()        cgi::main()            run_request()
                         └──────────────┬───────┴──────────────────┘
                                        ▼
                     run_request(Box<dyn Sapi>, Request) -> Outcome
                                        │   one fresh Vm per call
                                        ▼
 rhp-vm              Vm::new(sapi) → set_ini → set_request → start_request
                         → run_main → finish_request
                                        │
 rhp-builtins        main::sapi::{Sapi, Response, RequestInfo, header_op},
                     main::php_variables (treat_data), main::rfc1867 (multipart),
                     main::output
```

- **`rhp-builtins::sapi`** defines the contract: the `Sapi` trait, the
  `RequestInfo` a server fills in, and the `Response` header store, which
  `header()`, `setcookie()`, `http_response_code()` and the rest drive the
  way `sapi_header_op` does.
- **`rhp-vm::request`** turns a `RequestInfo` into `$_SERVER`, `$_GET`,
  `$_POST`, `$_COOKIE`, `$_FILES`, `$_ENV` and `$_REQUEST`, reads POST
  bodies (`post_max_size`, `enable_post_data_reading`, multipart), and sends
  the headers lazily.
- **`rhp-sapi`** is the library entry point (`run_request`, and
  `compile_file`, the one parse-and-compile path for scripts, includes,
  eval and preload) plus the shipped SAPIs: `CliSapi` (php-cli, with
  `cli::main` porting `php_cli.c`'s option table and passes),
  `cgi::CgiSapi` (php-cgi, with `cgi::main` porting `cgi_main.c`'s argv
  and environment handling) and
  `CaptureSapi`, which keeps a request's output in memory for tests and
  tools (`CaptureSapi::cli()` behaves as php-cli, `CaptureSapi::web(name)`
  as a web server).
- **`rhp-cli`** only picks the SAPI (`rhp --cgi …`, or the binary named
  `php-cgi`) and hands the command line to `cli::main` or `cgi::main`.
  Both parse it with a port of php's `php_getopt`, so bad options get
  php's messages (`-v` and `-i` run after module startup, so its
  warnings come first); the php-cli modes rhp lacks (`-l`, `-m`, `-s`,
  `-w`, `-B`/`-R`/`-F`/`-E`, `-S`, `--rf` and friends, `--ini`,
  `--repeat`) are refused after php's own checks, and `-a` gets the error
  of a php without readline.

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
| `write_raw_stdout` | a write to php-cli's `STDOUT` stream, past the output layer | `flush`, then stdout |
| `exits_on_hard_limit` | whether the hard memory limit ends the process (php-cli: exit 137) rather than just the request | `false` |

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

`Request::script` is a `Script::File` (its real path is `__FILE__`; it goes
through the compiled-script cache), a `Script::Source { path, bytes }` the
caller already holds (run as the file `path`, not cached), `Script::Code`
(`-r`) or `Script::Stdin`. `Request::new(script)` is a CLI-style request
with no settings or arguments.

### `Outcome`

`Exit(code)` (the script's exit status; 255 after a fatal error or a parse
error) or `CannotRead { path, error }` when the script file can't be read.
The HTTP status is in the `Response` that `send_headers` received, not in
`Outcome`.

### Lower level

`run_request` is a convenience over `Vm`. A server that needs to step
in between can drive it directly:
`Vm::new(sapi)`, `set_ini` for each setting, `set_request`, `start_request` (post-size check,
`X-Powered-By`), compile, `run_main` (script, shutdown functions,
destructors, output-buffer flush, headers if still unsent), and finally
`finish_request` (headers, `Sapi::flush`, upload cleanup). Dropping the
`Vm` also deletes this request's uploaded temp files and resets the
thread's request state (see *Threading model*). Compile the main
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
Like php (`is_cacheable_stream_path`), it caches `phar://` scripts as well
as plain files, revalidating them by the phar entry's mtime; scripts from
other stream wrappers are compiled on every include.

- **Per thread.** A `Program` holds `Rc`s, so the cache is a
  `thread_local!`: every request served on a worker thread shares it, as
  php-fpm workers share their shared-memory segment. Once bytecode is
  `Arc`-based it can become process-wide behind a lock; only the module
  touches it (bytecode.md §3.7 lists what that needs).
- **Nothing per-request is cached.** A `Program` is immutable: the VM runs
  its protos as they are (bytecode.md §3.6) and keeps statics, class
  statics, constants and globals in the request's `Vm`, so they start
  fresh in every request. Loading a cached file copies none of its code;
  the request only builds its own entries for it (functions, linked
  classes).
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
  cache and `opcache.enable_file_override`.

## Request lifecycle

What happens inside one `run_request`, in php's order:

1. **Engine setup.** A fresh `Vm` for the SAPI: `Vm::new` resets the
   thread's request state, installs the SAPI name and ini defaults and
   defines `PHP_SAPI`. `Request::cwd` (`Vm::set_cwd`) sets the directory
   the script runs in (php-cgi's chdir to the script's directory).
   `-d`-style settings apply over them.
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

**One live `Vm` per OS thread, pinned to it for the request's lifetime;
values stay plain `Rc`.** Engine values (`Value`, arrays, objects, the
`Sapi` box) are deliberately neither `Send` nor `Sync`: reference counting
stays cheap and non-atomic, as in php's NTS build. Concurrency comes from
threads, never from sharing an engine:

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
  request-scoped builtin state) is in `thread_local!`s, like php's ZTS
  module globals. `Vm::new` resets it, and so does dropping the `Vm`
  (after its own values), so the engine a thread runs is the one that
  state belongs to and an idle thread keeps nothing of its last request:
  a second engine on the same thread while one runs (nested, or two
  requests multiplexed on one thread) would clobber it. A request keeps
  its thread from `Vm::new` until its `Vm` is dropped.
  [`request-state.md`](./request-state.md) lists every `thread_local!`
  and which ones each request resets. The prelude's classes are linked
  once per thread (at its second request) and copied into each `Vm`, with
  fresh static properties. `rhp-sapi`'s
  `concurrent_requests_on_worker_threads_are_isolated` and
  `tests/isolation.rs` run requests on several threads at once and one
  after another on one thread and check that none of this leaks.
- **Working directory and umask are the request's.** rhp never calls
  chdir(2) or umask(2) for a script (`rhp_builtins::zend::virtual_cwd`, php's ZTS
  virtual cwd): `chdir()`, `getcwd()` and `umask()` change and read the
  request's values, every relative path rhp hands the OS, a C library or
  a child process is taken from the request's directory first (stat,
  open, access and opendir start from a descriptor of it, so the kernel
  walks no more of the path than from a real working directory), and files
  are created with the request's umask (a new file's mode is fixed up when
  the request's umask allows bits the process's takes away). Each request
  starts in `Request::cwd`, else the process's directory, with the
  process's umask, read once when the first engine starts (on macOS by
  setting it to 0777 for a moment, so a file another thread creates then
  can only come out stricter). Child processes start in the request's
  directory with its umask; posix_spawn can't set a umask, so when the
  request's differs from the process's, the child is forked and set up by
  hand. A Unix socket bound by a relative path is bound in the request's
  directory and still reports the name it was given; one the directory
  would make too long for `sun_path` is left relative. `opcache.preload`
  and `opcache.blacklist_filename`, startup settings, stay relative to
  the process's directory, as in php.
- **When the directory moves.** If the request's directory is renamed or
  removed while the request is in it, the calls that start from its
  descriptor (stat, open, access, opendir, chdir) follow it, as from a
  real working directory, and `getcwd()` gives its new name, or false once
  it is gone. Other paths (rename, unlink, mkdir, include, the C
  libraries, a child's directory) are joined to the name it had, until
  `getcwd()` or `chdir()` notices the new one. `realpath('.')` in a
  removed directory is false; php's is `'.'`.
- **SAPI functions.** Each SAPI supplies its own functions, as php's
  `sapi_module.additional_functions` and a SAPI's own module do: php-cli
  has `cli_set_process_title` / `cli_get_process_title`, php-cgi its
  `cgi-fcgi` module's `getallheaders` and `apache_*` functions
  (rhp-builtins `sapi/`, a `SapiDef` per SAPI name). An embedding SAPI
  has none of them. `function_exists()`, `get_extension_funcs()`,
  `get_defined_functions()` and Reflection see the running SAPI's set.
  php-cli and php-cgi also have `dl()`, with php's checks and warnings;
  rhp cannot load php's C extensions, so a library that does load is
  refused as unsupported.
- **pcntl only under the CLI and CGI.** pcntl is a `cli_only` module
  (PHP_NEW_EXTENSION's `cli` class): its functions, constants and
  extension entry exist only under the command-line SAPIs
  (`main::sapi::is_command_line`), so an embedding server never gets
  process-wide signal handlers or `fork` from a script. This differs
  from php only for embed: php's static build adds a `cli`-class
  extension to the cli, cgi, phpdbg and embed SAPIs (not php-fpm).
- **Process-wide state.** `putenv()` changes stay in the request (an
  overlay its child processes get); the process's environment never
  changes. What belongs to the OS process: signal handlers (`pcntl`, CLI
  and CGI only), and the shared caches (builtin registry, TLS and HTTP
  connection pools in `curl`), which are read-only or locked. The
  descriptor table is the process's too: the pipes rhp makes for a child
  are close-on-exec until its spawn hands them over, and spawns don't
  overlap, so a child started for one request never holds another's pipe
  ends open (php leaves them inheritable; its children come from one
  thread).

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
Counting starts when the script starts (`Vm::run_main`; compiling the main
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
`str_repeat`, `str_pad`, `implode`, `wordwrap`, `array_fill`, `array_pad`,
`range`, `fread`, `stream_get_line`, `file_get_contents` (plain files),
`ftruncate` on `php://memory`/`php://temp` and `unserialize`.

Building the request's superglobals is counted too. When parsing the
body crosses the limit, the parsers stop where php 8.5's do (the
multipart parser checks after each step, `explode` after each piece:
`rhp_values::mem::has_oom`, php's `ZEND_HAS_OOM()`), and the fatal error
is raised in Unknown on line 0 before the script runs (php's
`sapi_handle_post`). The raw body itself is not counted: php keeps at most
2 MB of it in memory (`php://temp`) or streams it (multipart).

php's hard limit (twice `memory_limit`, or `memory_hard_limit_percent`)
kills php-cli with exit 137. rhp checks it where the VM or a builtin
allocates a size it knows first but php can't refuse: string concatenation
(`.`, `.=`, interpolation), a far string offset, `hash_pbkdf2`, and
while compiling: `rhp_values::mem::compiling` makes every new chunk or
huge block check it, as php's compiler allocates with emalloc. The
compiler then stops at its next top-level statement and the includer
reports the line (`IncludeError::HardLimit`); php names the line it was
compiling. php-cli exits as php does; any other SAPI ends just the
request with the fatal error.

What still escapes the limit, until more builtins are mirrored:

- A builtin without a pre-check (`str_replace`, `array_merge`,
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
- Data kept from one request to the next belongs inside
  `rhp_values::mem::uncounted`, as php's opcache memory is outside the
  request heap; otherwise the request that built it is charged and a
  later one credited when it is freed. The compiled-script cache compiles
  the programs it keeps, and changes its tables, that way.
- rhp's data structures are not php's: an array of ints takes about five
  times php's memory (an `IndexMap` entry is 40 bytes, and its entries
  grow to match the index table), strings about the same. Compiling takes
  two to three times php's memory at its peak: php's grammar check runs
  first (24 bytes a token), then mago-syntax's tree (16-byte `Expression`
  and `Statement` references to nodes of their own size, in 32 KiB
  chunks like php's AST arena) lives while the compiler runs. Scripts
  close to the limit
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
  disconnects (`connection_aborted()`).
- **FastCGI.** A `cgi-fcgi` front end that reads FastCGI records, builds a
  `RequestInfo` from the params, and writes stdout records through a
  `Sapi`. It reuses `cgi::CgiSapi`'s header logic and `php_sapi_name()`
  stays `"cgi-fcgi"`.

## Gaps

- The request body is read completely before `run_request`; there is no
  streaming reader yet.
- `max_execution_time` is enforced only under the command-line SAPIs
  (php-cli, php-cgi): like php's NTS build it uses one process-wide
  interval timer (`rhp_builtins::zend::timeout`), so threaded embedders
  get no timeout yet. `max_input_time` is not enforced.
- `auto_append_file` is ignored (`auto_prepend_file` runs).
- php.ini: php-cli loads the `-c` file (or `$PHPRC`'s) and the `.ini`
  files in `$PHP_INI_SCAN_DIR` (`Request::ini_files`); rhp has no built-in
  search path or scan directory, and php-cgi and embedders load none.
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
