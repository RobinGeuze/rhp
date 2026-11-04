# Request state outside the `Vm`

A thread runs one live `Vm` at a time, from `Vm::new` to `finish_request`
(see [`sapi.md`](./sapi.md), *Threading model*). State that builtins, the
compiler or the VM keep outside the `Vm` struct lives in `thread_local!`s,
as php's ZTS build keeps its module globals per thread. This is every one
of them, what it holds and how a new request gets a clean copy. Phase 8
turns the resets into a mandatory hook per module; until then
`rhp_builtins::reset_request_state` (called first thing in `Vm::new`) is
the list, and a new `thread_local!` must go into this table and, when it
holds request state, into that function.

Kinds:

- **request**: belongs to one request; reset when the next engine starts.
- **call**: lives only while one call runs (a guard pops it, or the caller
  takes it right after); empty between calls. Some are also cleared at
  request start, in case a call was cut short.
- **thread**: a cache or setup shared by every request on the thread; holds
  nothing a script can change or see.

## rhp-builtins

| where | static | kind | reset by |
|---|---|---|---|
| `vcwd.rs` | `CWD` (the request's working directory, and a descriptor of it), `UMASK` | request | `vcwd::reset_state` |
| `runtime_fns.rs` | `INI`, `INI_STARTUP` (ini overrides, `-d` values) | request | `runtime_fns::reset_state` |
| `runtime_fns.rs` | `STARTUP_DIAGS` | request | `runtime_fns::reset_state` |
| `opcache_fns.rs` | `ACCEL_LOG` (opcache startup messages) | request | `opcache_fns::reset_state` |
| `streams.rs` | `LAST_OS_ERR`, `WRAPPER_ERR` (the last failure's errno / wrapper message) | call | `streams::reset_state` |
| `fdstream.rs` | `DIAGS`, `HAS_DIAGS` (warnings of host-less stream operations) | call | `fdstream::reset_state` |
| `dump.rs` | `VIEWS` (`__debugInfo` / handler views while formatting) | call | `dump::clear_views` |
| `sort_fns.rs` | `OBJECT_POS` (objects' internal array pointers, by object id) | request | `sort_fns::reset_state` |
| `json_fns.rs` | `LAST_ERROR` | request | `json_fns::reset_state` |
| `preg_fns.rs` | `LAST_ERROR` | request | `preg_fns::reset_state` |
| `pcre/mod.rs` | `STATE` (contexts, JIT stack, `pcre.*` settings, pattern cache) | request (settings, cache); thread (contexts) | `pcre::reset` via `preg_fns::reset_state` |
| `strsearch_fns.rs` | `STRTOK` | request | `strsearch_fns::reset_state` |
| `stdext_fns.rs` | `MT` (mt_rand state) | request | `stdext_fns::reset_state` |
| `sysmisc_fns.rs` | `TITLE` (`cli_set_process_title`) | request | `sysmisc_fns::reset_state` |
| `php_fns.rs` | `ENV_OVERLAY` (`putenv` changes) | request | `php_fns::reset_state` |
| `sapi.rs` | `SAPI_NAME`, `SAPI_INI` | request | `sapi::reset_state`, then `set_sapi` in `Vm::new` |
| `ext/bcmath` | `DEFAULT_SCALE` | request | `runtime_fns::reset_state` |
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
| `ext/xml` | `LIVE` (parsers, to break their cycles) | request | `xml::reset_state` via `libxml::reset_state` |
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

## rhp-values, rhp-semantics

| where | static | kind | reset by |
|---|---|---|---|
| `fmt.rs` | `PRECISION`, `SERIALIZE_PRECISION` | request | `fmt::reset_precision` via `runtime_fns::reset_state` |
| `lib.rs` | `DROPPED_STREAMS` (resources whose last reference went) | request | `reset_dropped_streams` |
| `lib.rs` | `OBJECT_FREE_HOOK` (set by every `Vm::new`) | thread | — |
| `mem.rs` | `STATE` (memory counters, limit) | request | `mem::end_request` in `Vm::new` and `finish_request` |
| `ops.rs` (semantics) | `CMP_GUARD` (objects being compared) | call | — |

## rhp-compiler, rhp-vm

| where | static | kind | reset by |
|---|---|---|---|
| compiler `lib.rs` | `COMPILE_ERROR_LINE` (line of the last compile error, read right after) | call | — |
| compiler `prelude.rs` | `PRELUDE_CACHE` (the compiled prelude) | thread | — |
| vm `classes.rs` | `LINKED_PRELUDE`, `PRELUDE_LINKED_ONCE` | thread | — |
| vm `lvalue.rs` | `PATH_DIAGS` (warnings of a cursor walk) | call | `lvalue::reset_path_diags` in `Vm::new` |
| vm `opcache/mod.rs` | `CACHE` (the compiled-script cache) | thread, as opcache's shared memory | `opcache_reset()` |

The harness's `SNAPSHOTS` (tools/rhp-difftest `phpt.rs`) is a per-worker
cache of the phpt runner, not engine state.

## Process-wide state

Not per thread, so shared by every engine in the process:

- pcntl's signal queue and flags (`ext/pcntl`, atomics written from the
  signal handler). Signal handlers belong to the process; pcntl exists
  only under the CLI and CGI (`sapi::has_pcntl`).
- `OnceLock`s and `Once`s that compute something once: the builtin
  registry, signatures, the process's umask (`vcwd`), libxml2's process
  init and default entity loader, curl's and OpenSSL's global init.
- C libraries' own globals, set once to dispatchers that read the calling
  thread's state (see [`libraries.md`](./libraries.md), *Threading*).
