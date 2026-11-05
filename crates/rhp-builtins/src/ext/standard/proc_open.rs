//! ext/standard/proc_open.c: `proc_open`/`proc_close`/`proc_get_status`/
//! `proc_terminate`, and the child spawning the exec family and `popen`
//! share.
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

use crate::main::streams::errno;
use crate::zend::zpp::zpp_long;
use crate::{Host, Thrown};

pub(crate) fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

/// `Z_PARAM_PATH`-style string argument: coerced to string, rejecting NUL bytes.
pub(crate) fn zpp_path(
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
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
pub(crate) enum SpawnCmd<'a> {
    /// `/bin/sh -c <command>`.
    Shell(&'a [u8]),
    /// `posix_spawnp(argv[0], argv)` — no shell.
    Argv(&'a [Vec<u8>]),
}

/// One child descriptor: the child's fd number, the fd it is fed from, and
/// (pipes/sockets) the parent's end, which the child must not keep.
pub(crate) struct Desc {
    pub(crate) index: i32,
    pub(crate) childend: OwnedFd,
    pub(crate) parentend: Option<OwnedFd>,
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
pub(crate) static SPAWN_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// `posix_spawn` the command with php's file actions, in `cwd` (relative to
/// the request's working directory), else in that directory. Returns the
/// child pid or the spawn error number. A child gets the request's umask
/// when it isn't the process's ([`fork_exec`]).
pub(crate) fn spawn(
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
pub(crate) fn make_pipe() -> Option<(OwnedFd, OwnedFd)> {
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
pub(crate) fn waitpid_retry(pid: libc::pid_t, options: i32) -> (libc::pid_t, i32) {
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
pub(crate) fn close_status(st: i32) -> i64 {
    if libc::WIFEXITED(st) {
        libc::WEXITSTATUS(st) as i64
    } else {
        st as i64
    }
}

/// php wraps every `popen`ed pipe in a (transient) stream resource, which
/// consumes a resource id; mirror that so later ids line up.
pub(crate) fn consume_stream_id(host: &mut dyn Host, fd: &OwnedFd) {
    if let Some(d) = dup_fd(fd.as_raw_fd()) {
        if let Some(s) = host.stream_from_fd(d) {
            host.stream_close(&s);
        }
    }
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
                let s = crate::zend::zpp::zval_get_string(host, v)?;
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
                // _php_stream_cast: read-ahead the child cannot see
                let lost = host.stream_meta(&item).map_or(0, |m| m.unread_bytes);
                if lost > 0 {
                    crate::warn(
                        host,
                        "proc_open",
                        &format!("{lost} bytes of buffered data lost during stream conversion!"),
                    )?;
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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("proc_open", fn_proc_open),
            ("proc_close", fn_proc_close),
            ("proc_get_status", fn_proc_get_status),
            ("proc_terminate", fn_proc_terminate),
        ],
    );
}
