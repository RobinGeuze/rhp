# Request state outside the `Vm`

A thread runs one live `Vm` at a time, from `Vm::new` until the `Vm` is
dropped (see [`sapi.md`](./sapi.md), *Threading model*). State that
builtins, the compiler or the VM keep outside the `Vm` struct lives in
`thread_local!`s, as php's ZTS build keeps its module globals per thread.
This is every one of them, what it holds and how a request gets a clean
copy. Every module (`ModuleDef`, `module.rs`) states its reset hook,
`no_state` when it has none, so a module cannot forget one: a new
`thread_local!` must go into this table and, when it holds request state,
into its module's `reset` (a SAPI's state into its `SapiDef`'s).
`rhp_builtins::reset_request_state` (`registry.rs`) runs every module's
reset in module order, then every SAPI module's, then
`rhp_values::reset_dropped_streams`; no reset may depend on another's. rhp-vm's `lifecycle::reset_request_state` runs it, with the VM's
own resets, first thing in `Vm::new` and again when the `Vm` drops (after
its values), so an idle thread holds nothing of its last request.

Kinds:

- **request**: belongs to one request; reset when its engine is dropped and
  again when the next one starts.
- **call**: lives only while one call runs (a guard pops it, or the caller
  takes it right after); empty between calls. Some are also cleared at
  request start, in case a call was cut short.
- **thread**: a cache or setup shared by every request on the thread; holds
  nothing a script can change or see.
- **startup**: filled before a request starts, also by the preload engine
  that runs first, and emptied by the request's start; neither a new
  engine nor the preload one going away may drop it.

## rhp-builtins

| where | static | kind | reset by |
|---|---|---|---|
| `zend/virtual_cwd.rs` | `CWD` (the request's working directory, and a descriptor of it), `UMASK` | request | `virtual_cwd::reset_state` |
| `zend/call_stack.rs` | `BASE`, `LIMIT`, `RESERVED` (the stack limit; a fiber switch swaps `BASE` and `LIMIT`) | request | set by `call_stack::init` at request start |
| `zend/fibers.rs` | `BLOCKING` (fiber switches blocked while tick functions or signal handlers run) | call | `SwitchBlock` guards |
| `zend/virtual_cwd/realpath_cache.rs` | `CACHE` (php's realpath cache and its settings; php keeps it per process) | request | `realpath_cache::reset_state` |
| `main/ini.rs` | `INI`, `INI_STARTUP` (ini overrides, `-d` values) | request | `ini::reset_state` |
| `main/fopen_wrappers.rs` | `BASEDIR`, `ACTIVE` (open_basedir, its entries expanded after an ini_set) | request | `fopen_wrappers::reset_state` |
| `main/ini.rs` | `STARTUP_DIAGS` | request | `ini::reset_state` |
| `ext/opcache/mod.rs` | `ACCEL_LOG` (opcache's startup messages) | startup | emptied by `start_request` |
| `main/streams/mod.rs` | `LAST_OS_ERR`, `WRAPPER_ERR` (the last failure's errno / wrapper message) | call | `streams::reset_state` |
| `main/streams/plain_wrapper.rs` | `DIAGS`, `HAS_DIAGS` (warnings of host-less stream operations) | call | `plain_wrapper::reset_state` |
| `ext/standard/var.rs` | `VIEWS` (`__debugInfo` / handler views while formatting) | call | `var::clear_views` |
| `ext/standard/var.rs` | `DUMPING` (the arrays and objects `var_dump` / `debug_zval_dump` are inside of: php's recursion guards) | call | emptied as each dump returns |
| `ext/standard/array.rs` | `OBJECT_POS` (objects' internal array pointers, by object id) | request | `array::reset_state` |
| `main/streams/xp_socket.rs` | `BOUND_UNIX` (Unix socket paths bound relative to the request's directory, and the names given) | request | `xp_socket::reset_state` |
| `ext/json/mod.rs` | `LAST_ERROR` | request | `json::reset_state` |
| `ext/json/mod.rs` | `SERIALIZING` (objects whose `jsonSerialize()` runs: php's JSON guard) | call | emptied as each call returns |
| `ext/pcre/php_pcre.rs` | `LAST_ERROR` | request | `php_pcre::reset_state` |
| `ext/pcre/mod.rs` | `STATE` (contexts, JIT stack, `pcre.*` settings, pattern cache) | request (settings, cache); thread (contexts) | `pcre::reset` via `php_pcre::reset_state` |
| `ext/standard/string.rs` | `STRTOK` | request | `string::reset_state` |
| `ext/random/mod.rs` | `MT` (mt_rand state) | request | `random::reset_state` |
| `sapi/cli.rs` | `TITLE` (`cli_set_process_title`) | request | `cli::reset_state` (php-cli's `SapiDef` reset, run whatever the SAPI) |
| `ext/standard/basic_functions.rs` | `ENV_OVERLAY` (`putenv` changes) | request | `basic_functions::reset_state` |
| `ext/standard/basic_functions.rs` | `TICK_FUNCS` (`register_tick_function`) | request | `basic_functions::reset_state` |
| `ext/standard/basic_functions.rs` | `ORIG_TZ` (the TZ `putenv` replaced in the process environment) | process, as in php | `basic_functions::reset_state` restores it |
| `ext/standard/string.rs` | `LOCALE_CHANGED` (the C library's locale, set by `setlocale`) | process, as in php | `string::reset_state` resets the locale |
| `ext/standard/url_scanner_ex.rs` | `STATE` (the URL rewriter) | request | `url_scanner_ex::reset_state` |
| `ext/standard/browscap.rs` | `DATA` (the parsed browscap file) | thread (php: process) | — |
| `main/sapi.rs` | `SAPI_NAME`, `SAPI_DEF` (its `SapiDef` and bit), `SAPI_INI` | request | `sapi::reset_state`, then `set_sapi` in `Vm::new` |
| `ext/bcmath` | `DEFAULT_SCALE` | request | `bcmath::reset_state` |
| `ext/bz2` | `BZ_ERRORS` | request | `bz2::reset_state` |
| `ext/curl/mod.rs` | `EASY`, `MULTI`, `SHARE` (handle tables) | request | `curl::reset_state` |
| `ext/curl/share.rs` | `PERSISTENT` (php's `CURL_G(persistent_curlsh)`) | thread, as in php | — |
| `ext/date` | `TIMEZONE`, `TZCACHE`, `LAST_ERRORS` | request | `date::reset_state` |
| `ext/date` | `CALLBACK_PANIC` | call | — |
| `ext/gmp` | `RAND` | request | `gmp::reset_state` |
| `ext/iconv/sys.rs` | `ERRNO` | request | `iconv::reset_state` |
| `ext/iconv/sys.rs` | `THREAD_LOCALE` (a C.UTF-8 `locale_t`) | thread | — |
| `ext/intl/mod.rs` | `ERR` (`intl_get_error_*`) | request | `intl::error_reset` |
| `ext/intl/grapheme.rs` | `GLOBAL` (break iterator, cloned per call) | thread | — |
| `ext/libxml/mod.rs` | `STATE` (errors, stream context, entity loader) | request | `libxml::reset_state` |
| `ext/libxml/mod.rs` | `INSTALLED` (php's handlers set on this thread) | thread | — |
| `ext/libxml/ffi.rs` | `FRAMES` (hosts of running libxml2 calls) | call | — |
| `ext/xml` | `LIVE` (parsers, to break their cycles) | request | `xml::reset_state` |
| `ext/mbstring` | `STATE` | request | `mbstring::reset_state` |
| `ext/openssl` | `NATIVES` (keys, certificates by object id) | request | `openssl::reset_state` |
| `ext/openssl` | `ERRORS`, `ERRORS_MARK` (`openssl_error_string` ring) | request (php: per worker) | `openssl::reset_state` |
| `ext/openssl` | `PENDING` (an exception thrown mid-call) | call | `openssl::reset_state` |
| `ext/openssl` | `CALLBACK_PANIC` | call | — |
| `ext/openssl` | `CUSTOM_LIBCTX` | thread | — |
| `ext/sockets` | `LAST_ERROR` (`socket_last_error`) | request (php: per worker) | `sockets::reset_state` |

`ERRORS` and the sockets `LAST_ERROR` are module globals php never resets
between requests (GINIT): a php-fpm worker's next request can still read
them. rhp resets them, so each request starts as a fresh php process does.

## rhp-values

| where | static | kind | reset by |
|---|---|---|---|
| `fmt.rs` | `PRECISION`, `SERIALIZE_PRECISION` | request | `fmt::reset_precision` via rhp-builtins' `ini::reset_state` |
| `lib.rs` | `DROPPED_STREAMS` (resources whose last reference went) | request | `reset_dropped_streams` |
| `lib.rs` | `OBJECT_FREE_HOOK` (set by every `Vm::new`) | thread | — |
| `mem.rs` | `STATE` (memory counters, limit) | request | `mem::end_request` in `Vm::new` and `finish_request` |
| `ops.rs` | `CMP_GUARD` (objects being compared) | call | — |

## rhp-compiler, rhp-vm

| where | static | kind | reset by |
|---|---|---|---|
| compiler `lib.rs` | `COMPILE_ERROR_LINE` (line of the last compile error, read right after) | call | — |
| compiler `lib.rs` | `COMPILE_ERROR_DIAGS` (diagnostics raised before that error, taken right after) | call | — |
| compiler `prelude.rs` | `PRELUDE_CACHE` (the compiled prelude) | thread | — |
| vm `classes.rs` | `LINKED_PRELUDE`, `PRELUDE_LINKED_ONCE` | thread | — |
| vm `lvalue.rs` | `PATH_DIAGS` (warnings of a cursor walk) | call | `lvalue::reset_path_diags` via `lifecycle::reset_request_state` |
| vm `opcache/mod.rs` | `CACHE` (the compiled-script cache) | thread, as opcache's shared memory | `opcache_reset()` |

The harness's `SNAPSHOTS` (tools/rhp-difftest `phpt.rs`) is a per-worker
cache of the phpt runner, not engine state.

## Process-wide state

Not per thread, so shared by every engine in the process:

- pcntl's signal queue and flags (`ext/pcntl`, atomics written from the
  signal handler). Signal handlers belong to the process; pcntl exists
  only under the CLI and CGI (`main::sapi::is_command_line`).
- `OnceLock`s and `Once`s that compute something once: the module list,
  the builtin registry, signatures, the process's umask (`virtual_cwd`),
  libxml2's process init and default entity loader, curl's and OpenSSL's
  global init.
- C libraries' own globals, set once to dispatchers that read the calling
  thread's state (see [`libraries.md`](./libraries.md), *Threading*).
