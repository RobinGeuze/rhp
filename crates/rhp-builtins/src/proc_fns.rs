//! Program execution (php's `ext/standard/proc_open.c` + `exec.c`) and
//! `stream_select` (`streamsfuncs.c`): `proc_open`/`proc_close`/
//! `proc_get_status`/`proc_terminate`/`proc_nice`, the `exec`/`system`/
//! `passthru`/`shell_exec` family, `escapeshellarg`/`escapeshellcmd`, plus a
//! few process-environment helpers (`putenv`, `get_current_user`,
//! `gethostname`, `gethostbyname`, `usleep`, `cli_set_process_title`).
//!
//! Children are started with `posix_spawn(3)` exactly as php does when it is
//! built with `USE_POSIX_SPAWN` (macOS, glibc ≥ 2.29): a string command runs as
//! `/bin/sh -c <command>`, an array command via `posix_spawnp` with no shell.
//! Pipe parent ends become `stream` resources backed by the VM's fd streams.
//! A child starts in the request's working directory (`crate::zend::virtual_cwd`), as
//! php's ZTS build starts it in its virtual one, and with the request's
//! umask: when that differs from the process's, which posix_spawn can't
//! change, the child is forked and set up by hand instead.
//!
//! (php's warnings — e.g. "posix_spawn() failed: …" — are not emitted: this
//! engine has no diagnostics channel yet; the return values match.)

use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::streams::errno;
use crate::{Host, Thrown};

fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

/// php's `zend_parse_arg_long_weak` (`Z_PARAM_LONG`, coercive mode): ints
/// pass, bools/null become 0/1/0, integral-range floats and fully numeric
/// strings are truncated; anything else is a TypeError
/// "`fname(): Argument #n ($pname) must be of type int, T given`".
pub(crate) fn zpp_long(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<i64, Thrown> {
    let v = deref(&args[i]);
    let float_to_long = |f: f64| -> Option<i64> {
        // ZEND_DOUBLE_FITS_LONG: !(d >= 2^63 || d < -2^63), and not NaN.
        const TWO_63: f64 = 9_223_372_036_854_775_808.0;
        if f.is_nan() || f >= TWO_63 || f < -TWO_63 {
            None
        } else {
            Some(f as i64)
        }
    };
    let n = match &v {
        Value::Int(n) => Some(*n),
        Value::Bool(b) => Some(*b as i64),
        Value::Null => Some(0),
        Value::Float(f) => float_to_long(*f),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Some(n),
            Some(conv::Num::Float(f)) => float_to_long(f),
            None => None,
        },
        _ => None,
    };
    n.ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type int, {} given",
            i + 1,
            crate::zend::zpp::zval_value_name(host, &v)
        ))
    })
}

/// `Z_PARAM_STR` (coercive): scalars become strings; arrays, objects without
/// `__toString` and resources are a TypeError.
pub(crate) fn zpp_string(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let v = deref(&args[i]);
    match &v {
        Value::Str(s) => Ok(s.as_bytes().to_vec()),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null => {
            Ok(conv::to_string_bytes(&v))
        }
        _ => Err(Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type string, {} given",
            i + 1,
            crate::zend::zpp::zval_value_name(host, &v)
        ))),
    }
}

/// `Z_PARAM_PATH`-style string argument: coerced to string, rejecting NUL bytes.
fn path_arg(args: &[Value], i: usize, fname: &str, pname: &str) -> Result<Vec<u8>, Thrown> {
    let s = conv::to_string_bytes(&deref(&args[i]));
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(s)
}

fn cstr(b: &[u8]) -> CString {
    CString::new(b.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default()
}

/// The current process environment as `KEY=VALUE` entries (php passes
/// `environ` when no `$env_vars` array is given) with the request's
/// `putenv` changes applied (php's putenv edits `environ` itself).
fn current_environ() -> Vec<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let mut env: Vec<(Vec<u8>, Vec<u8>)> = std::env::vars_os()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect();
    for (k, v) in crate::env_overrides() {
        env.retain(|(n, _)| *n != k);
        if let Some(v) = v {
            env.push((k, v));
        }
    }
    env.into_iter()
        .map(|(k, v)| {
            let mut e = k;
            e.push(b'=');
            e.extend_from_slice(&v);
            e
        })
        .collect()
}

// ---------------------------------------------------------------------------
// spawning
// ---------------------------------------------------------------------------

/// What to execute.
enum SpawnCmd<'a> {
    /// `/bin/sh -c <command>`.
    Shell(&'a [u8]),
    /// `posix_spawnp(argv[0], argv)` — no shell.
    Argv(&'a [Vec<u8>]),
}

/// One child descriptor: the child's fd number, the fd it is fed from, and
/// (pipes/sockets) the parent's end, which the child must not keep.
struct Desc {
    index: i32,
    childend: OwnedFd,
    parentend: Option<OwnedFd>,
}

/// One file action of the spawn, in php's order.
#[derive(Clone, Copy)]
enum Action {
    Close(RawFd),
    Dup2(RawFd, i32),
}

/// Taken for writing while a child is spawned, and for reading while a
/// descriptor is made close-on-exec in two steps (where the OS has no flag
/// for it): a child never inherits a descriptor another thread is still
/// setting up, such as the far end of its pipes, which would hold off their
/// EOF until that child exits.
static SPAWN_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// `posix_spawn` the command with php's file actions, in `cwd` (relative to
/// the request's working directory), else in that directory. Returns the
/// child pid or the spawn error number. A child gets the request's umask
/// when it isn't the process's ([`fork_exec`]).
fn spawn(
    cmd: SpawnCmd,
    descs: &[Desc],
    cwd: Option<&[u8]>,
    env: Option<&[Vec<u8>]>,
) -> Result<libc::pid_t, i32> {
    let mut actions = Vec::new();
    let mut inherit = Vec::new();
    for d in descs {
        if let Some(p) = &d.parentend {
            actions.push(Action::Close(p.as_raw_fd()));
        }
        let c = d.childend.as_raw_fd();
        if c != d.index {
            actions.push(Action::Dup2(c, d.index));
            actions.push(Action::Close(c));
        } else if unsafe { libc::fcntl(c, libc::F_GETFD) } & libc::FD_CLOEXEC != 0 {
            // Already at the right number: it must survive exec.
            inherit.push(c);
        }
    }
    let cwd_c = match cwd {
        Some(dir) => Some(cstr(&crate::zend::virtual_cwd::resolve(dir))),
        None => crate::zend::virtual_cwd::child_cwd().map(|d| cstr(&d)),
    };
    let env_vec = match env {
        Some(e) => e.to_vec(),
        None => current_environ(),
    };
    let env_c: Vec<CString> = env_vec.iter().map(|e| cstr(e)).collect();
    let mut envp: Vec<*mut libc::c_char> = env_c
        .iter()
        .map(|c| c.as_ptr() as *mut libc::c_char)
        .collect();
    envp.push(std::ptr::null_mut());
    let (args_c, search) = match cmd {
        SpawnCmd::Shell(command) => (vec![cstr(b"sh"), cstr(b"-c"), cstr(command)], false),
        SpawnCmd::Argv(argv) => (argv.iter().map(|a| cstr(a)).collect(), true),
    };
    let mut argp: Vec<*mut libc::c_char> = args_c.iter().map(|c| c.as_ptr() as *mut _).collect();
    argp.push(std::ptr::null_mut());
    let program = if search {
        args_c[0].clone()
    } else {
        cstr(b"/bin/sh")
    };

    let _spawning = SPAWN_LOCK.write().unwrap_or_else(|e| e.into_inner());
    let cloexec = |fd: RawFd, on: libc::c_int| unsafe { libc::fcntl(fd, libc::F_SETFD, on) };
    for &fd in &inherit {
        cloexec(fd, 0);
    }
    let r = match crate::zend::virtual_cwd::child_umask() {
        None => unsafe { posix_spawn(&program, search, &actions, cwd_c.as_deref(), &argp, &envp) },
        Some(mask) => {
            let candidates = if search {
                exec_candidates(args_c[0].as_bytes())
            } else {
                vec![program]
            };
            unsafe { fork_exec(&candidates, &actions, cwd_c.as_deref(), mask, &argp, &envp) }
        }
    };
    for &fd in &inherit {
        cloexec(fd, libc::FD_CLOEXEC);
    }
    r
}

/// `posix_spawn` (`posix_spawnp` when `search`) with `actions`.
unsafe fn posix_spawn(
    program: &CStr,
    search: bool,
    actions: &[Action],
    cwd: Option<&CStr>,
    argv: &[*mut libc::c_char],
    envp: &[*mut libc::c_char],
) -> Result<libc::pid_t, i32> {
    let mut fa: libc::posix_spawn_file_actions_t = std::mem::zeroed();
    libc::posix_spawn_file_actions_init(&mut fa);
    let mut r = 0;
    for a in actions {
        let e = match *a {
            Action::Close(fd) => libc::posix_spawn_file_actions_addclose(&mut fa, fd),
            Action::Dup2(from, to) => libc::posix_spawn_file_actions_adddup2(&mut fa, from, to),
        };
        if e != 0 {
            r = e;
        }
    }
    if let Some(c) = cwd {
        libc::posix_spawn_file_actions_addchdir_np(&mut fa, c.as_ptr());
    }
    let mut pid: libc::pid_t = 0;
    if r == 0 {
        let spawn = if search {
            libc::posix_spawnp
        } else {
            libc::posix_spawn
        };
        r = spawn(
            &mut pid,
            program.as_ptr(),
            &fa,
            std::ptr::null(),
            argv.as_ptr(),
            envp.as_ptr(),
        );
    }
    libc::posix_spawn_file_actions_destroy(&mut fa);
    if r != 0 {
        return Err(r);
    }
    Ok(pid)
}

/// Where `posix_spawnp` looks for `file`: the process's `PATH`, unless the
/// name holds a slash.
fn exec_candidates(file: &[u8]) -> Vec<CString> {
    use std::os::unix::ffi::OsStrExt;
    if file.contains(&b'/') {
        return vec![cstr(file)];
    }
    let path = std::env::var_os("PATH");
    let path = path
        .as_ref()
        .map_or(&b"/usr/bin:/bin"[..], |p| p.as_bytes());
    path.split(|&b| b == b':')
        .map(|dir| {
            let mut full = dir.to_vec();
            if !full.is_empty() {
                full.push(b'/');
            }
            full.extend_from_slice(file);
            cstr(&full)
        })
        .collect()
}

/// What `posix_spawn` does, by hand, for a child that needs the request's
/// umask (posix_spawn has no attribute for one): fork, then `actions`, the
/// directory, the umask and exec (each of `candidates` in turn, as
/// `posix_spawnp` searches). Between fork and exec the child makes only
/// async-signal-safe calls and allocates nothing; a failure there comes
/// back through a close-on-exec pipe as the spawn's error, as from
/// posix_spawn.
unsafe fn fork_exec(
    candidates: &[CString],
    actions: &[Action],
    cwd: Option<&CStr>,
    mask: u32,
    argv: &[*mut libc::c_char],
    envp: &[*mut libc::c_char],
) -> Result<libc::pid_t, i32> {
    // The spawn lock is held: a plain pipe() can't leak to another child.
    // Its ends move above every descriptor number the child dup2s onto.
    let mut report = [0 as libc::c_int; 2];
    if libc::pipe(report.as_mut_ptr()) != 0 {
        return Err(errno());
    }
    let above = actions
        .iter()
        .filter_map(|a| match *a {
            Action::Dup2(_, to) => Some(to + 1),
            Action::Close(_) => None,
        })
        .fold(3, i32::max);
    let moved = report.map(|fd| {
        let high = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, above);
        libc::close(fd);
        high
    });
    if moved.iter().any(|&fd| fd < 0) {
        moved.iter().filter(|&&fd| fd >= 0).for_each(|&fd| {
            libc::close(fd);
        });
        return Err(libc::EMFILE);
    }
    let (rd, wr) = (
        OwnedFd::from_raw_fd(moved[0]),
        OwnedFd::from_raw_fd(moved[1]),
    );
    // The child must not run the parent's signal handlers before exec.
    let mut all: libc::sigset_t = std::mem::zeroed();
    let mut old: libc::sigset_t = std::mem::zeroed();
    libc::sigfillset(&mut all);
    libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut old);
    let pid = libc::fork();
    if pid == 0 {
        let fail = |e: i32| -> ! {
            let bytes = e.to_ne_bytes();
            libc::write(wr.as_raw_fd(), bytes.as_ptr().cast(), bytes.len());
            libc::_exit(127)
        };
        let nsig = if cfg!(target_os = "linux") { 65 } else { 32 };
        for sig in 1..nsig {
            let mut sa: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, std::ptr::null(), &mut sa) == 0
                && sa.sa_sigaction != libc::SIG_IGN
                && sa.sa_sigaction != libc::SIG_DFL
            {
                sa.sa_sigaction = libc::SIG_DFL;
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
        for a in actions {
            match *a {
                Action::Close(fd) => {
                    libc::close(fd);
                }
                Action::Dup2(from, to) => {
                    if libc::dup2(from, to) < 0 {
                        fail(errno());
                    }
                }
            }
        }
        if let Some(dir) = cwd {
            if libc::chdir(dir.as_ptr()) != 0 {
                fail(errno());
            }
        }
        libc::umask(mask as libc::mode_t);
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
        let mut err = libc::ENOENT;
        let mut denied = false;
        for c in candidates {
            libc::execve(c.as_ptr(), argv.as_ptr().cast(), envp.as_ptr().cast());
            err = errno();
            match err {
                libc::EACCES => denied = true,
                libc::ENOENT | libc::ENOTDIR | libc::ESTALE | libc::ENODEV | libc::ETIMEDOUT => {}
                _ => fail(err),
            }
        }
        fail(if denied { libc::EACCES } else { err });
    }
    let forked = errno();
    libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
    drop(wr);
    if pid < 0 {
        return Err(forked);
    }
    let mut bytes = [0u8; 4];
    let n = loop {
        let n = libc::read(rd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len());
        if n < 0 && errno() == libc::EINTR {
            continue;
        }
        break n;
    };
    if n == 4 {
        waitpid_retry(pid, 0);
        return Err(i32::from_ne_bytes(bytes));
    }
    Ok(pid)
}

/// A `pipe(2)` pair `(read, write)`, both ends close-on-exec: the child
/// gets its end through the spawn's dup2, and no other child inherits
/// either (php's ends are inheritable; its children are in one thread).
fn make_pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    #[cfg(target_os = "linux")]
    let r = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    let r = {
        let _setup = SPAWN_LOCK.read().unwrap_or_else(|e| e.into_inner());
        let r = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if r == 0 {
            fds.iter().for_each(|&fd| set_cloexec_raw(fd));
        }
        r
    };
    if r != 0 {
        return None;
    }
    unsafe { Some((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

/// A connected `AF_UNIX` stream pair, both ends close-on-exec (see
/// [`make_pipe`]).
fn make_socketpair() -> Option<(OwnedFd, OwnedFd)> {
    let mut sv = [0 as libc::c_int; 2];
    #[cfg(target_os = "linux")]
    let r = unsafe {
        let ty = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
        libc::socketpair(libc::AF_UNIX, ty, 0, sv.as_mut_ptr())
    };
    #[cfg(not(target_os = "linux"))]
    let r = {
        let _setup = SPAWN_LOCK.read().unwrap_or_else(|e| e.into_inner());
        let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, sv.as_mut_ptr()) };
        if r == 0 {
            sv.iter().for_each(|&fd| set_cloexec_raw(fd));
        }
        r
    };
    if r != 0 {
        return None;
    }
    unsafe { Some((OwnedFd::from_raw_fd(sv[0]), OwnedFd::from_raw_fd(sv[1]))) }
}

fn set_cloexec_raw(fd: RawFd) {
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFD);
        if fl >= 0 {
            libc::fcntl(fd, libc::F_SETFD, fl | libc::FD_CLOEXEC);
        }
    }
}

/// A close-on-exec `dup` of `raw`.
fn dup_fd(raw: RawFd) -> Option<OwnedFd> {
    let d = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
    (d >= 0).then(|| unsafe { OwnedFd::from_raw_fd(d) })
}

/// `waitpid` retrying on EINTR.
fn waitpid_retry(pid: libc::pid_t, options: i32) -> (libc::pid_t, i32) {
    loop {
        let mut st = 0;
        let r = unsafe { libc::waitpid(pid, &mut st, options) };
        if r == -1 && errno() == libc::EINTR {
            continue;
        }
        return (r, st);
    }
}

/// The value php's `pclose`-style close reports: the exit code when the child
/// exited normally, else the raw wait status.
fn close_status(st: i32) -> i64 {
    if libc::WIFEXITED(st) {
        libc::WEXITSTATUS(st) as i64
    } else {
        st as i64
    }
}

/// `popen(cmd, "r")`: run `/bin/sh -c cmd` with its stdout on a pipe; stdin
/// and stderr are inherited. Returns (read end, pid).
fn popen_read(cmd: &[u8]) -> Option<(OwnedFd, libc::pid_t)> {
    let (r, w) = make_pipe()?;
    let descs = [Desc {
        index: 1,
        childend: w,
        parentend: None,
    }];
    let pid = spawn(SpawnCmd::Shell(cmd), &descs, None, None).ok()?;
    drop(descs);
    Some((r, pid))
}

/// php wraps every `popen`ed pipe in a (transient) stream resource, which
/// consumes a resource id; mirror that so later ids line up.
fn consume_stream_id(host: &mut dyn Host, fd: &OwnedFd) {
    if let Some(d) = dup_fd(fd.as_raw_fd()) {
        if let Some(s) = host.stream_from_fd(d) {
            host.stream_close(&s);
        }
    }
}

fn read_fd_to_end(fd: &OwnedFd, mut each: impl FnMut(&[u8])) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = unsafe {
            libc::read(
                fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n > 0 {
            each(&buf[..n as usize]);
        } else if n == 0 || errno() != libc::EINTR {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// exec / system / passthru / shell_exec
// ---------------------------------------------------------------------------

/// php's `strip_trailing_whitespace` (C `isspace`).
fn strip_trailing_ws(b: &[u8]) -> &[u8] {
    let mut l = b.len();
    while l > 0 && matches!(b[l - 1], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        l -= 1;
    }
    &b[..l]
}

/// `php_exec`: mode 0 = exec (last line), 1 = system (echo lines), 2 = exec
/// with an output array, 3 = passthru (raw). Returns (return value, status).
fn php_exec(host: &mut dyn Host, mode: u8, cmd: &[u8], out: &mut Option<PhpArray>) -> (Value, i64) {
    let Some((fd, pid)) = popen_read(cmd) else {
        return (Value::Bool(false), -1);
    };
    consume_stream_id(host, &fd);
    let mut ret = Value::str(Vec::new());
    if mode == 3 {
        read_fd_to_end(&fd, |chunk| host.write_out(chunk));
    } else {
        let mut data = Vec::new();
        read_fd_to_end(&fd, |chunk| data.extend_from_slice(chunk));
        let mut last: Option<&[u8]> = None;
        let mut start = 0;
        while start < data.len() {
            let end = data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| start + i + 1)
                .unwrap_or(data.len());
            let line = &data[start..end];
            match mode {
                1 => host.write_out(line),
                2 => {
                    if let Some(a) = out.as_mut() {
                        a.push(Value::str(strip_trailing_ws(line).to_vec()));
                    }
                }
                _ => {}
            }
            last = Some(line);
            start = end;
        }
        if let Some(l) = last {
            ret = Value::str(strip_trailing_ws(l).to_vec());
        }
    }
    drop(fd);
    let (r, st) = waitpid_retry(pid, 0);
    let status = if r <= 0 { -1 } else { close_status(st) };
    (ret, status)
}

fn exec_family(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let cmd = path_arg(args, 0, fname, "command")?;
    if cmd.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #1 ($command) must not be empty"),
        ));
    }
    let (mode, code_idx) = match fname {
        "exec" => (if args.len() > 1 { 2 } else { 0 }, 2),
        "system" => (1, 1),
        _ => (3, 1),
    };
    let mut out = None;
    if mode == 2 {
        out = Some(match deref(&args[1]) {
            Value::Array(a) => a,
            _ => PhpArray::new(),
        });
    }
    let (ret, status) = php_exec(host, mode, &cmd, &mut out);
    if let Some(a) = out {
        write_ref(args.get(1), Value::Array(a));
    }
    if args.len() > code_idx {
        write_ref(args.get(code_idx), Value::Int(status));
    }
    // passthru returns null on success (php 8.2+), false on failure.
    if mode == 3 {
        return Ok(if matches!(ret, Value::Bool(false)) {
            ret
        } else {
            Value::Null
        });
    }
    Ok(ret)
}

fn fn_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    exec_family(host, args, "exec")
}

fn fn_system(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    exec_family(host, args, "system")
}

fn fn_passthru(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    exec_family(host, args, "passthru")
}

fn fn_shell_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cmd = path_arg(args, 0, "shell_exec", "command")?;
    if cmd.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "shell_exec(): Argument #1 ($command) must not be empty",
        ));
    }
    let Some((fd, pid)) = popen_read(&cmd) else {
        return Ok(Value::Bool(false));
    };
    consume_stream_id(host, &fd);
    let mut data = Vec::new();
    read_fd_to_end(&fd, |c| data.extend_from_slice(c));
    drop(fd);
    waitpid_retry(pid, 0);
    Ok(if data.is_empty() {
        Value::Null
    } else {
        Value::str(data)
    })
}

// ---------------------------------------------------------------------------
// escapeshellarg / escapeshellcmd
// ---------------------------------------------------------------------------

/// `php_mblen` under php's default `C.UTF-8` LC_CTYPE: the length of the valid
/// UTF-8 sequence at the start of `s`, or -1 when it is invalid/incomplete.
fn mblen(s: &[u8]) -> isize {
    let b = s[0];
    if b < 0x80 {
        return 1;
    }
    let n = match b {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return -1,
    };
    if s.len() < n {
        return -1;
    }
    match std::str::from_utf8(&s[..n]) {
        Ok(_) => n as isize,
        Err(_) => -1,
    }
}

fn escape_shell_arg(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 2);
    out.push(b'\'');
    let mut x = 0;
    while x < s.len() {
        let l = mblen(&s[x..]);
        if l < 0 {
            x += 1;
            continue;
        }
        if l > 1 {
            out.extend_from_slice(&s[x..x + l as usize]);
            x += l as usize;
            continue;
        }
        if s[x] == b'\'' {
            out.extend_from_slice(b"'\\'");
        }
        out.push(s[x]);
        x += 1;
    }
    out.push(b'\'');
    out
}

fn escape_shell_cmd(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    // Position of the matching closing quote while inside a quoted span.
    let mut p: Option<usize> = None;
    let mut x = 0;
    while x < s.len() {
        let l = mblen(&s[x..]);
        if l < 0 {
            x += 1;
            continue;
        }
        if l > 1 {
            out.extend_from_slice(&s[x..x + l as usize]);
            x += l as usize;
            continue;
        }
        let c = s[x];
        match c {
            b'"' | b'\'' => {
                if p.is_none() {
                    p = s[x + 1..].iter().position(|&b| b == c).map(|i| x + 1 + i);
                    if p.is_none() {
                        out.push(b'\\');
                    }
                } else if p.map(|i| s[i]) == Some(c) {
                    p = None;
                } else {
                    out.push(b'\\');
                }
                out.push(c);
            }
            b'#' | b'&' | b';' | b'`' | b'|' | b'*' | b'?' | b'~' | b'<' | b'>' | b'^' | b'('
            | b')' | b'[' | b']' | b'{' | b'}' | b'$' | b'\\' | b'\n' | 0xFF => {
                out.push(b'\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        x += 1;
    }
    out
}

fn fn_escapeshellarg(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = path_arg(args, 0, "escapeshellarg", "arg")?;
    Ok(Value::str(escape_shell_arg(&s)))
}

fn fn_escapeshellcmd(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = path_arg(args, 0, "escapeshellcmd", "command")?;
    Ok(Value::str(escape_shell_cmd(&s)))
}

// ---------------------------------------------------------------------------
// proc_open & friends
// ---------------------------------------------------------------------------

/// A `process` resource's payload.
struct ProcHandle {
    pid: libc::pid_t,
    command: Vec<u8>,
    /// The pipe stream resources handed back in `$pipes` (closed by proc_close).
    pipes: Vec<Value>,
    /// The cached final wait status, once observed (`WIFEXITED` only).
    cached: Option<i32>,
}

impl ProcHandle {
    fn waitpid_cached(&mut self, options: i32) -> (libc::pid_t, i32) {
        if let Some(st) = self.cached {
            return (self.pid, st);
        }
        let (r, st) = waitpid_retry(self.pid, options);
        if r > 0 && libc::WIFEXITED(st) {
            self.cached = Some(st);
        }
        (r, st)
    }
}

/// Children of `popen()` streams, by stream resource id.
struct PopenState(std::collections::HashMap<u32, std::process::Child>);

fn popen_state(host: &mut dyn Host) -> Option<&mut PopenState> {
    host.module_state("popen", || Box::new(PopenState(Default::default())))?
        .downcast_mut::<PopenState>()
}

/// `popen($command, $mode)`: `/bin/sh -c` with its stdout (mode "r") or
/// stdin ("w") as a pipe stream.
fn fn_popen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::process::{Command, Stdio};
    let cmd = path_arg(args, 0, "popen", "command")?;
    let mode = path_arg(args, 1, "popen", "mode")?;
    let write = match mode.first() {
        Some(b'r') if !mode.contains(&b'+') => false,
        Some(b'w') if !mode.contains(&b'+') => true,
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "popen(): Argument #2 ($mode) must be one of \"r\", \"rb\", \"w\", or \"wb\"",
            ))
        }
    };
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(std::ffi::OsStr::from_bytes(&cmd));
    if let Some(dir) = crate::zend::virtual_cwd::child_cwd() {
        c.current_dir(std::ffi::OsStr::from_bytes(&dir));
    }
    if let Some(mask) = crate::zend::virtual_cwd::child_umask() {
        use std::os::unix::process::CommandExt;
        // umask(2) is async-signal-safe.
        unsafe {
            c.pre_exec(move || {
                libc::umask(mask as libc::mode_t);
                Ok(())
            })
        };
    }
    if write {
        c.stdin(Stdio::piped());
    } else {
        c.stdout(Stdio::piped());
    }
    let spawned = {
        let _spawning = SPAWN_LOCK.write().unwrap_or_else(|e| e.into_inner());
        c.spawn()
    };
    let mut child = match spawned {
        Ok(ch) => ch,
        Err(e) => {
            crate::warn(host, "popen", &format!("{e}"))?;
            return Ok(Value::Bool(false));
        }
    };
    let fd: OwnedFd = if write {
        child.stdin.take().map(OwnedFd::from)
    } else {
        child.stdout.take().map(OwnedFd::from)
    }
    .expect("piped");
    let Some(stream) = host.stream_from_fd(fd) else {
        return Ok(Value::Bool(false));
    };
    if let (Value::Resource(r), Some(st)) = (&stream, popen_state(host)) {
        st.0.insert(r.id, child);
    }
    Ok(stream)
}

/// `pclose($handle)`: close the pipe and wait; the exit status.
fn fn_pclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = deref(&args[0]);
    let id = match &h {
        Value::Resource(r) => r.id,
        other => {
            return Err(Thrown::type_error(format!(
                "pclose(): Argument #1 ($handle) must be of type resource, {} given",
                crate::zend::zpp::zval_value_name(host, other)
            )))
        }
    };
    host.stream_close(&h);
    let child = popen_state(host).and_then(|st| st.0.remove(&id));
    let Some(mut child) = child else {
        return Ok(Value::Int(-1));
    };
    Ok(Value::Int(match child.wait() {
        Ok(st) => {
            use std::os::unix::process::ExitStatusExt;
            st.code()
                .map(|c| c as i64)
                .unwrap_or_else(|| st.into_raw() as i64)
        }
        Err(_) => -1,
    }))
}

/// proc_open_rsrc_dtor for a process resource whose last reference went
/// away: close the pipes, then a non-blocking reap.
pub(crate) fn proc_dtor(host: &mut dyn Host, data: Box<dyn std::any::Any>) {
    let Ok(mut h) = data.downcast::<ProcHandle>() else {
        return;
    };
    for p in std::mem::take(&mut h.pipes) {
        host.stream_close(&p);
    }
    h.waitpid_cached(libc::WNOHANG);
}

fn proc_arg<'h>(
    host: &'h mut dyn Host,
    args: &[Value],
    fname: &str,
) -> Result<&'h mut ProcHandle, Thrown> {
    let v = deref(&args[0]);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($process) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    }
    let is_proc = matches!(&v, Value::Resource(r) if r.type_name == "process");
    match host.resource_get(&v) {
        Some(d) if is_proc => d
            .downcast_mut::<ProcHandle>()
            .ok_or_else(|| bad_proc(fname)),
        _ => Err(bad_proc(fname)),
    }
}

fn bad_proc(fname: &str) -> Thrown {
    Thrown::type_error(format!(
        "{fname}(): supplied resource is not a valid process resource"
    ))
}

fn fn_proc_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = proc_arg(host, args, "proc_get_status")?;
    let (mut running, mut signaled, mut stopped) = (true, false, false);
    let (mut exitcode, mut termsig, mut stopsig) = (-1i64, 0i64, 0i64);
    let (r, st) = h.waitpid_cached(libc::WNOHANG | libc::WUNTRACED);
    if r == h.pid {
        if libc::WIFEXITED(st) {
            running = false;
            exitcode = libc::WEXITSTATUS(st) as i64;
        }
        if libc::WIFSIGNALED(st) {
            running = false;
            signaled = true;
            termsig = libc::WTERMSIG(st) as i64;
        }
        if libc::WIFSTOPPED(st) {
            stopped = true;
            stopsig = libc::WSTOPSIG(st) as i64;
        }
    } else if r == -1 {
        running = false;
    }
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    };
    put("command", Value::str(h.command.clone()));
    put("pid", Value::Int(h.pid as i64));
    put("cached", Value::Bool(h.cached.is_some()));
    put("running", Value::Bool(running));
    put("signaled", Value::Bool(signaled));
    put("stopped", Value::Bool(stopped));
    put("exitcode", Value::Int(exitcode));
    put("termsig", Value::Int(termsig));
    put("stopsig", Value::Int(stopsig));
    Ok(Value::Array(a))
}

fn fn_proc_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pipes = std::mem::take(&mut proc_arg(host, args, "proc_close")?.pipes);
    // Close all pipe handles first to avoid a deadlock.
    for p in &pipes {
        host.stream_close(p);
    }
    let mut h = host
        .resource_remove(&deref(&args[0]))
        .and_then(|b| b.downcast::<ProcHandle>().ok())
        .ok_or_else(|| bad_proc("proc_close"))?;
    let (r, st) = h.waitpid_cached(0);
    Ok(Value::Int(if r <= 0 { -1 } else { close_status(st) }))
}

fn fn_proc_terminate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    proc_arg(host, args, "proc_terminate")?;
    let sig = match args.get(1) {
        Some(_) => zpp_long(host, args, 1, "proc_terminate", "signal")?,
        None => libc::SIGTERM as i64,
    };
    let h = proc_arg(host, args, "proc_terminate")?;
    Ok(Value::Bool(unsafe { libc::kill(h.pid, sig as i32) } == 0))
}

fn fn_proc_nice(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pri = zpp_long(host, args, 0, "proc_nice", "priority")?;
    unsafe {
        *libc_errno() = 0;
        libc::nice(pri as i32);
        Ok(Value::Bool(*libc_errno() == 0))
    }
}

#[cfg(target_os = "macos")]
unsafe fn libc_errno() -> *mut i32 {
    libc::__error()
}

#[cfg(not(target_os = "macos"))]
unsafe fn libc_errno() -> *mut i32 {
    libc::__errno_location()
}

/// Open a `['file', $path, $mode]` descriptor target with fopen-mode flags.
fn open_for_mode(path: &[u8], mode: &[u8]) -> Option<OwnedFd> {
    let plus = mode.contains(&b'+');
    let acc = if plus {
        libc::O_RDWR
    } else if mode.first() == Some(&b'r') {
        libc::O_RDONLY
    } else {
        libc::O_WRONLY
    };
    let extra = match mode.first()? {
        b'r' => 0,
        b'w' => libc::O_CREAT | libc::O_TRUNC,
        b'a' => libc::O_CREAT | libc::O_APPEND,
        b'x' => libc::O_CREAT | libc::O_EXCL,
        b'c' => libc::O_CREAT,
        _ => return None,
    };
    let p = cstr(path.strip_prefix(b"file://").unwrap_or(path));
    crate::zend::virtual_cwd::open(p.as_bytes(), acc | extra, 0o666).ok()
}

/// Element `i` of a descriptor array as a string, or the php ValueError.
fn desc_string(ht: &PhpArray, i: i64, what: &str) -> Result<Vec<u8>, Thrown> {
    match ht.get(&ArrayKey::Int(i)) {
        Some(v) => Ok(conv::to_string_bytes(&deref(v))),
        None => Err(Thrown::error("ValueError", format!("Missing {what}"))),
    }
}

/// What a descriptor became (for building `$pipes`).
enum DescKind {
    Std,
    Pipe,
}

fn fn_proc_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // --- the command -------------------------------------------------------
    let cmd_v = deref(&args[0]);
    let mut argv: Option<Vec<Vec<u8>>> = None;
    let command: Vec<u8> = match &cmd_v {
        Value::Array(a) => {
            if a.is_empty() {
                return Err(Thrown::error(
                    "ValueError",
                    "proc_open(): Argument #1 ($command) must not be empty",
                ));
            }
            let mut v = Vec::new();
            for (i, (_, el)) in a.iter().enumerate() {
                let s = conv::to_string_bytes(&deref(el));
                if i == 0 && s.is_empty() {
                    return Err(Thrown::error(
                        "ValueError",
                        "First element must contain a non-empty program name",
                    ));
                }
                if s.contains(&0) {
                    return Err(Thrown::error(
                        "ValueError",
                        format!("Command array element {} contains a null byte", i + 1),
                    ));
                }
                v.push(s);
            }
            let first = v[0].clone();
            argv = Some(v);
            first
        }
        Value::Null | Value::Object(_) | Value::Closure(_) | Value::Resource(_) => {
            return Err(Thrown::type_error(format!(
                "proc_open(): Argument #1 ($command) must be of type array|string, {} given",
                crate::zend::zpp::zval_value_name(host, &cmd_v)
            )))
        }
        other => conv::to_string_bytes(other),
    };

    // --- the descriptor spec ------------------------------------------------
    let spec = match deref(&args[1]) {
        Value::Array(a) => a,
        other => {
            return Err(Thrown::type_error(format!(
                "proc_open(): Argument #2 ($descriptor_spec) must be of type array, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    let cwd: Option<Vec<u8>> = match args.get(3).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_string_bytes(&v)),
    };
    let env: Option<Vec<Vec<u8>>> = match args.get(4).map(deref) {
        Some(Value::Array(a)) => {
            let mut e: Vec<Vec<u8>> = Vec::new();
            for (k, v) in a.iter() {
                let s = conv::to_string_bytes(&deref(v));
                if s.is_empty() {
                    continue;
                }
                match k {
                    ArrayKey::Str(ks) if !ks.as_bytes().is_empty() => {
                        let mut entry = ks.as_bytes().to_vec();
                        entry.push(b'=');
                        entry.extend_from_slice(&s);
                        e.push(entry);
                    }
                    _ => e.push(s),
                }
            }
            Some(e)
        }
        _ => None,
    };

    let mut descs: Vec<Desc> = Vec::new();
    let mut kinds: Vec<DescKind> = Vec::new();
    for (k, item) in spec.iter() {
        let index =
            match k {
                ArrayKey::Int(n) => *n as i32,
                ArrayKey::Str(_) => return Err(Thrown::error(
                    "ValueError",
                    "proc_open(): Argument #2 ($descriptor_spec) must be an integer indexed array",
                )),
            };
        let item = deref(item);
        let (childend, parentend, kind) = match &item {
            Value::Resource(r) => {
                if r.type_name != "stream" && r.type_name != "persistent stream" {
                    return Err(Thrown::type_error(
                        "proc_open(): supplied resource is not a valid stream resource",
                    ));
                }
                match host.stream_dup_fd(&item) {
                    Ok(fd) => {
                        set_cloexec_raw(fd.as_raw_fd());
                        (fd, None, DescKind::Std)
                    }
                    // php: "Cannot represent a stream of type X as a File Descriptor"
                    Err(_) => return Ok(Value::Bool(false)),
                }
            }
            Value::Array(ht) => {
                let ty = desc_string(ht, 0, "handle qualifier")?;
                match ty.as_slice() {
                    b"pipe" => {
                        let mode = desc_string(ht, 1, "mode parameter for 'pipe'")?;
                        let Some((r, w)) = make_pipe() else {
                            return Ok(Value::Bool(false));
                        };
                        let (parent, child) = if mode.first() == Some(&b'w') {
                            (r, w)
                        } else {
                            (w, r)
                        };
                        (child, Some(parent), DescKind::Pipe)
                    }
                    b"socket" => {
                        let Some((p, c)) = make_socketpair() else {
                            return Ok(Value::Bool(false));
                        };
                        (c, Some(p), DescKind::Pipe)
                    }
                    b"file" => {
                        let path = desc_string(ht, 1, "file name parameter for 'file'")?;
                        let mode = desc_string(ht, 2, "mode parameter for 'file'")?;
                        let Some(fd) = open_for_mode(&path, &mode) else {
                            return Ok(Value::Bool(false));
                        };
                        // php opens this through a (transient) stream resource,
                        // which consumes a resource id.
                        consume_stream_id(host, &fd);
                        (fd, None, DescKind::Std)
                    }
                    b"redirect" => {
                        let target = match ht.get(&ArrayKey::Int(1)).map(deref) {
                            None => {
                                return Err(Thrown::error(
                                    "ValueError",
                                    "Missing redirection target",
                                ))
                            }
                            Some(Value::Int(n)) => n as i32,
                            Some(other) => {
                                return Err(Thrown::error(
                                    "ValueError",
                                    format!(
                                        "Redirection target must be of type int, {} given",
                                        crate::zend::zpp::zval_value_name(host, &other)
                                    ),
                                ))
                            }
                        };
                        let from = descs
                            .iter()
                            .find(|d| d.index == target)
                            .map(|d| d.childend.as_raw_fd());
                        let from = match from {
                            Some(f) => f,
                            None if (0..=2).contains(&target) => target,
                            // php: "Redirection target %d not found"
                            None => return Ok(Value::Bool(false)),
                        };
                        let Some(fd) = dup_fd(from) else {
                            return Ok(Value::Bool(false));
                        };
                        (fd, None, DescKind::Std)
                    }
                    b"null" => {
                        let p = cstr(b"/dev/null");
                        let fd = unsafe { libc::open(p.as_ptr(), libc::O_RDWR) };
                        if fd < 0 {
                            return Ok(Value::Bool(false));
                        }
                        (unsafe { OwnedFd::from_raw_fd(fd) }, None, DescKind::Std)
                    }
                    // `pty` (openpty) is not supported here; anything else is
                    // "… is not a valid descriptor spec/mode".
                    _ => return Ok(Value::Bool(false)),
                }
            }
            _ => return Err(Thrown::error(
                "ValueError",
                "proc_open(): Argument #2 ($descriptor_spec) must only contain arrays and streams",
            )),
        };
        descs.push(Desc {
            index,
            childend,
            parentend,
        });
        kinds.push(kind);
    }

    // --- spawn ----------------------------------------------------------------
    let cmd = match &argv {
        Some(a) => SpawnCmd::Argv(a),
        None => SpawnCmd::Shell(&command),
    };
    let pid = match spawn(cmd, &descs, cwd.as_deref(), env.as_deref()) {
        Ok(p) => p,
        // php: "posix_spawn() failed: <strerror>"
        Err(_) => return Ok(Value::Bool(false)),
    };

    // --- parent side: close child ends, wrap parent ends as streams ----------
    let mut pipes_arr = PhpArray::new();
    let mut pipes = Vec::new();
    for (d, kind) in descs.into_iter().zip(kinds) {
        drop(d.childend);
        if let (DescKind::Pipe, Some(p)) = (kind, d.parentend) {
            if let Some(s) = host.stream_from_fd(p) {
                pipes_arr.insert(ArrayKey::Int(d.index as i64), s.clone());
                pipes.push(s);
            }
        }
    }
    write_ref(args.get(2), Value::Array(pipes_arr));
    let h = ProcHandle {
        pid,
        command,
        pipes,
        cached: None,
    };
    Ok(host.resource_insert("process", Box::new(h)))
}

// ---------------------------------------------------------------------------
// stream_select
// ---------------------------------------------------------------------------

/// A by-ref array argument of stream_select (None when null).
fn select_array(v: Option<&Value>) -> Option<PhpArray> {
    match v.map(deref) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

fn fn_stream_select(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arrays = [
        select_array(args.first()),
        select_array(args.get(1)),
        select_array(args.get(2)),
    ];
    let sec = match args.get(3).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_int(&v)),
    };
    let usec = match args.get(4).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_int(&v)),
    };

    let mut sets: [libc::fd_set; 3] = unsafe { std::mem::zeroed() };
    for s in sets.iter_mut() {
        unsafe { libc::FD_ZERO(s) };
    }
    let mut max_fd: RawFd = 0;
    let mut nsets = 0;
    // php's `php_stream_from_zval_no_verify` on an element that is not a
    // stream raises a TypeError but carries on: the select still runs (and
    // rewrites the arrays) before the exception surfaces.
    let mut pending: Option<Thrown> = None;
    for (i, a) in arrays.iter().enumerate() {
        if let Some(a) = a {
            for (_, el) in a.iter() {
                let el = deref(el);
                if let Some((fd, _)) = host.stream_select_fd(&el) {
                    if (fd as usize) < libc::FD_SETSIZE {
                        unsafe { libc::FD_SET(fd, &mut sets[i]) };
                        max_fd = max_fd.max(fd);
                        nsets += 1;
                    }
                } else if !matches!(el, Value::Resource(_)) {
                    pending.get_or_insert_with(|| {
                        Thrown::type_error(
                            "stream_select(): supplied argument is not a valid stream resource",
                        )
                    });
                } else if let Some(m) = host.stream_meta(&el) {
                    crate::warn(
                        host,
                        "stream_select",
                        &format!(
                            "Cannot represent a stream of type {} as a select()able descriptor",
                            m.stream_type
                        ),
                    )?;
                } else {
                    // A closed stream or another kind of resource.
                    pending.get_or_insert_with(|| {
                        Thrown::type_error(
                            "stream_select(): supplied resource is not a valid stream resource",
                        )
                    });
                }
            }
        }
    }
    // (php chains the TypeError as the ValueError's previous exception.)
    if nsets == 0 {
        return Err(Thrown::error("ValueError", "No stream arrays were passed"));
    }
    if sec.is_none() && usec.is_some_and(|u| u != 0) {
        return Err(Thrown::error(
            "ValueError",
            "stream_select(): Argument #5 ($microseconds) must be null when argument #4 ($seconds) is null",
        ));
    }
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut tvp: *mut libc::timeval = std::ptr::null_mut();
    if let Some(sec) = sec {
        let usec = usec.unwrap_or(0);
        if sec < 0 {
            return Err(Thrown::error(
                "ValueError",
                "stream_select(): Argument #4 ($seconds) must be greater than or equal to 0",
            ));
        }
        if usec < 0 {
            return Err(Thrown::error(
                "ValueError",
                "stream_select(): Argument #5 ($microseconds) must be greater than or equal to 0",
            ));
        }
        tv.tv_sec = (sec + usec / 1_000_000) as libc::time_t;
        tv.tv_usec = (usec % 1_000_000) as libc::suseconds_t;
        tvp = &mut tv;
    }

    // Streams with buffered unread data count as readable immediately.
    if let Some(r) = &arrays[0] {
        let mut ready = PhpArray::new();
        for (k, el) in r.iter() {
            if let Some((_, true)) = host.stream_select_fd(&deref(el)) {
                ready.insert(k.clone(), el.clone());
            }
        }
        if !ready.is_empty() {
            let n = ready.len() as i64;
            write_ref(args.first(), Value::Array(ready));
            for (i, a) in arrays.iter().enumerate().skip(1) {
                if a.is_some() {
                    write_ref(args.get(i), Value::Array(PhpArray::new()));
                }
            }
            return pending.map_or(Ok(Value::Int(n)), Err);
        }
    }

    let (rp, rest) = sets.split_at_mut(1);
    let (wp, ep) = rest.split_at_mut(1);
    // No retry on EINTR: php reports an interrupted select (a signal
    // arrived) as a warning and false, so the handler runs right after.
    let n = unsafe { libc::select(max_fd + 1, &mut rp[0], &mut wp[0], &mut ep[0], tvp) };
    if n == -1 {
        let e = errno();
        crate::warn(
            host,
            "stream_select",
            &format!(
                "Unable to select [{e}]: {} (max_fd={max_fd})",
                crate::streams::strerror(e)
            ),
        )?;
        return pending.map_or(Ok(Value::Bool(false)), Err);
    }
    for (i, a) in arrays.iter().enumerate() {
        let Some(a) = a else { continue };
        let mut kept = PhpArray::new();
        for (k, el) in a.iter() {
            if let Some((fd, _)) = host.stream_select_fd(&deref(el)) {
                if (fd as usize) < libc::FD_SETSIZE && unsafe { libc::FD_ISSET(fd, &sets[i]) } {
                    kept.insert(k.clone(), el.clone());
                }
            }
        }
        write_ref(args.get(i), Value::Array(kept));
    }
    pending.map_or(Ok(Value::Int(n as i64)), Err)
}

/// `stream_isatty($stream)`: isatty(2) on the stream's fd.
fn fn_stream_isatty(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    Ok(Value::Bool(match host.stream_select_fd(&v) {
        Some((fd, _)) => unsafe { libc::isatty(fd) == 1 },
        None => false,
    }))
}

// ---------------------------------------------------------------------------
// process environment helpers
// ---------------------------------------------------------------------------

/// The passwd entry for a uid: (name, passwd, uid, gid, gecos, dir, shell).
pub(crate) struct Passwd {
    pub name: Vec<u8>,
    pub passwd: Vec<u8>,
    pub uid: u32,
    pub gid: u32,
    pub gecos: Vec<u8>,
    pub dir: Vec<u8>,
    pub shell: Vec<u8>,
}

unsafe fn cbytes(p: *const libc::c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        std::ffi::CStr::from_ptr(p).to_bytes().to_vec()
    }
}

fn passwd_from(pw: &libc::passwd) -> Passwd {
    unsafe {
        Passwd {
            name: cbytes(pw.pw_name),
            passwd: cbytes(pw.pw_passwd),
            uid: pw.pw_uid,
            gid: pw.pw_gid,
            gecos: cbytes(pw.pw_gecos),
            dir: cbytes(pw.pw_dir),
            shell: cbytes(pw.pw_shell),
        }
    }
}

pub(crate) fn getpwuid(uid: u32) -> Option<Passwd> {
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16384];
        let mut res: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut res);
        (r == 0 && !res.is_null()).then(|| passwd_from(&pw))
    }
}

pub(crate) fn getpwnam(name: &[u8]) -> Option<Passwd> {
    let c = CString::new(name.to_vec()).ok()?;
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16384];
        let mut res: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut res);
        (r == 0 && !res.is_null()).then(|| passwd_from(&pw))
    }
}

fn fn_usleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let us = zpp_long(host, args, 0, "usleep", "microseconds")?;
    if us < 0 {
        return Err(Thrown::error(
            "ValueError",
            "usleep(): Argument #1 ($microseconds) must be greater than or equal to 0",
        ));
    }
    // php calls usleep(3) once: a signal cuts the sleep short.
    unsafe { libc::usleep(us as libc::useconds_t) };
    Ok(Value::Null)
}

fn fn_sleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = zpp_long(host, args, 0, "sleep", "seconds")?;
    if s < 0 {
        return Err(Thrown::error(
            "ValueError",
            "sleep(): Argument #1 ($seconds) must be greater than or equal to 0",
        ));
    }
    // php's sleep(3): returns the unslept seconds when a signal interrupts.
    Ok(Value::Int(unsafe { libc::sleep(s as libc::c_uint) } as i64))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("proc_open", fn_proc_open),
            ("proc_close", fn_proc_close),
            ("popen", fn_popen),
            ("pclose", fn_pclose),
            ("proc_get_status", fn_proc_get_status),
            ("proc_terminate", fn_proc_terminate),
            ("proc_nice", fn_proc_nice),
            ("exec", fn_exec),
            ("system", fn_system),
            ("passthru", fn_passthru),
            ("shell_exec", fn_shell_exec),
            ("escapeshellarg", fn_escapeshellarg),
            ("escapeshellcmd", fn_escapeshellcmd),
            ("stream_select", fn_stream_select),
            ("stream_isatty", fn_stream_isatty),
            ("usleep", fn_usleep),
            ("sleep", fn_sleep),
        ],
    );
}
