//! ext/standard/exec.c: `exec`, `system`, `passthru`, `shell_exec`,
//! `escapeshellarg`/`escapeshellcmd` and `proc_nice`, over the spawning in
//! [`super::proc_open`].

use std::os::fd::{AsRawFd, OwnedFd};

use rhp_values::ops::deref;
use rhp_values::{PhpArray, Value};

use crate::ext::standard::proc_open::{
    close_status, consume_stream_id, make_pipe, spawn, waitpid_retry, write_ref, zpp_path, Desc,
    SpawnCmd,
};
use crate::main::streams::errno;
use crate::zend::zpp::zpp_long;
use crate::{Host, Thrown};

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
    let cmd = zpp_path(args, 0, fname, "command")?;
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
    let cmd = zpp_path(args, 0, "shell_exec", "command")?;
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
    let s = zpp_path(args, 0, "escapeshellarg", "arg")?;
    Ok(Value::str(escape_shell_arg(&s)))
}

fn fn_escapeshellcmd(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = zpp_path(args, 0, "escapeshellcmd", "command")?;
    Ok(Value::str(escape_shell_cmd(&s)))
}

fn fn_proc_nice(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pri = zpp_long(host, args, 0, "proc_nice", "priority")?;
    unsafe {
        *libc_errno() = 0;
        libc::nice(pri as i32);
        if *libc_errno() == 0 {
            return Ok(Value::Bool(true));
        }
    }
    crate::warn(
        host,
        "proc_nice",
        "Only a super user may attempt to increase the priority of a process",
    )?;
    Ok(Value::Bool(false))
}

#[cfg(target_os = "macos")]
unsafe fn libc_errno() -> *mut i32 {
    libc::__error()
}

#[cfg(not(target_os = "macos"))]
unsafe fn libc_errno() -> *mut i32 {
    libc::__errno_location()
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("proc_nice", fn_proc_nice),
            ("exec", fn_exec),
            ("system", fn_system),
            ("passthru", fn_passthru),
            ("shell_exec", fn_shell_exec),
            ("escapeshellarg", fn_escapeshellarg),
            ("escapeshellcmd", fn_escapeshellcmd),
        ],
    );
}
