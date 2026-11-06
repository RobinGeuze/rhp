//! ext/standard/mail.c: `mail()`, which pipes the message to the
//! sendmail_path command (`/bin/sh -c`, like php's popen).

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::standard::proc_open::{make_pipe, spawn, waitpid_retry, zpp_path, Desc, SpawnCmd};
use crate::main::ini::{ini_parse_bool, ini_value};
use crate::{Host, Thrown};

/// The current value of directive `name`, None when null.
fn ini(name: &[u8]) -> Option<Vec<u8>> {
    ini_value(name).flatten()
}

/// php_mail_build_headers_check_field_value: why a header value is refused.
fn check_field_value(v: &[u8]) -> Option<&'static str> {
    let at = |i: usize| v.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < v.len() {
        match v[i] {
            b'\r' => {
                if at(i + 1) != b'\n' {
                    return Some("contains CR character that is not allowed in the header");
                }
                if v.len() - i >= 3 && matches!(at(i + 2), b' ' | b'\t') {
                    i += 3;
                    continue;
                }
                return Some(
                    "contains CRLF characters that are used as a line separator and are not allowed in the header",
                );
            }
            b'\n' => {
                if v.len() - i >= 2 && matches!(at(i + 1), b' ' | b'\t') {
                    i += 2;
                    continue;
                }
                return Some("contains LF character that is not allowed in the header");
            }
            0 => return Some("contains NULL character that is not allowed in the header"),
            _ => i += 1,
        }
    }
    None
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `%s` of a C string: up to the first NUL.
fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// php_mail_build_headers_elem.
fn build_elem(host: &dyn Host, s: &mut Vec<u8>, key: &[u8], val: &Value) -> Result<(), Thrown> {
    match val {
        Value::Str(v) => {
            let v = v.as_bytes();
            if key.iter().any(|&c| !(33..=126).contains(&c) || c == b':') {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Header name \"{}\" contains invalid characters",
                        lossy(cstr(key))
                    ),
                ));
            }
            if let Some(why) = check_field_value(v) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("Header \"{}\" {why}", lossy(cstr(key))),
                ));
            }
            s.extend_from_slice(key);
            s.extend_from_slice(b": ");
            s.extend_from_slice(cstr(v));
            s.extend_from_slice(b"\r\n");
            Ok(())
        }
        Value::Array(a) => build_elems(host, s, key, a),
        _ => Err(Thrown::type_error(format!(
            "Header \"{}\" must be of type array|string, {} given",
            lossy(cstr(key)),
            crate::zend::zpp::zval_value_name(host, val)
        ))),
    }
}

/// php_mail_build_headers_elems.
fn build_elems(host: &dyn Host, s: &mut Vec<u8>, key: &[u8], a: &PhpArray) -> Result<(), Thrown> {
    for (k, v) in a.iter() {
        if let ArrayKey::Str(k) = k {
            return Err(Thrown::type_error(format!(
                "Header \"{}\" must only contain numeric keys, \"{}\" found",
                lossy(cstr(key)),
                lossy(cstr(k.as_bytes()))
            )));
        }
        let v = deref(v);
        if !matches!(v, Value::Str(_)) {
            return Err(Thrown::type_error(format!(
                "Header \"{}\" must only contain values of type string, {} found",
                lossy(cstr(key)),
                crate::zend::zpp::zval_value_name(host, &v)
            )));
        }
        build_elem(host, s, key, &v)?;
    }
    Ok(())
}

/// php_mail_build_headers.
fn build_headers(host: &dyn Host, headers: &PhpArray) -> Result<Vec<u8>, Thrown> {
    let mut s = Vec::new();
    for (k, v) in headers.iter() {
        let key = match k {
            ArrayKey::Int(i) => {
                return Err(Thrown::type_error(format!(
                    "Header name cannot be numeric, {i} given"
                )))
            }
            ArrayKey::Str(k) => k.as_bytes(),
        };
        let v = deref(v);
        let lk = key.to_ascii_lowercase();
        match lk.as_slice() {
            b"to" | b"subject" => {
                let name = if lk == b"to" { "To" } else { "Subject" };
                return Err(Thrown::error(
                    "ValueError",
                    format!("The additional headers cannot contain the \"{name}\" header"),
                ));
            }
            b"orig-date" | b"from" | b"sender" | b"reply-to" | b"cc" | b"bcc" | b"message-id"
            | b"references" | b"in-reply-to"
                if matches!(v, Value::Array(_)) =>
            {
                return Err(Thrown::type_error(format!(
                    "Header \"{}\" must be of type string, array given",
                    lossy(&lk)
                )));
            }
            _ => build_elem(host, &mut s, key, &v)?,
        }
    }
    s.truncate(s.len().saturating_sub(2));
    Ok(s)
}

/// The `to` / `subject` clean-up: trailing whitespace off, control
/// characters to spaces except a folded line's CRLF + whitespace.
fn clean_field(f: &[u8]) -> Vec<u8> {
    let mut r = f.to_vec();
    while r
        .last()
        .is_some_and(|c| c.is_ascii_whitespace() || *c == 0x0b)
    {
        r.pop();
    }
    let at = |r: &[u8], i: usize| r.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < r.len() {
        if r[i] < 0x20 || r[i] == 0x7f {
            if r[i] == b'\r' && at(&r, i + 1) == b'\n' && matches!(at(&r, i + 2), b' ' | b'\t') {
                i += 2;
                while matches!(at(&r, i + 1), b' ' | b'\t') {
                    i += 1;
                }
                i += 1;
                continue;
            }
            r[i] = b' ';
        }
        i += 1;
    }
    r
}

/// php_mail_detect_multiple_crlf.
fn multiple_crlf(h: &[u8]) -> bool {
    let h = cstr(h);
    if h.is_empty() {
        return false;
    }
    if h[0] < 33 || h[0] > 126 || h[0] == b':' {
        return true;
    }
    let at = |i: usize| h.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < h.len() {
        match h[i] {
            b'\r' => {
                if matches!(at(i + 1), 0 | b'\r')
                    || (at(i + 1) == b'\n' && matches!(at(i + 2), 0 | b'\n' | b'\r'))
                {
                    return true;
                }
                i += 2;
            }
            b'\n' => {
                if matches!(at(i + 1), 0 | b'\r' | b'\n') {
                    return true;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    false
}

/// php_mail_log_to_file.
fn log_to_file(host: &mut dyn Host, file: &[u8], msg: &[u8]) -> Result<(), Thrown> {
    let Some(url) = crate::locate_for_open(host, "mail", file)? else {
        return Ok(());
    };
    crate::main::streams::clear_errno();
    match host.stream_open(&url, b"a") {
        Some(res) => {
            host.stream_write(&res, msg);
            host.stream_close(&res);
        }
        None => crate::open_failed(host, "mail", file)?,
    }
    Ok(())
}

/// php_mail: log, build the header block, run sendmail_path and report its
/// exit status.
fn php_mail(
    host: &mut dyn Host,
    to: &[u8],
    subject: &[u8],
    message: &[u8],
    headers: Option<&[u8]>,
    extra_cmd: Option<&[u8]>,
) -> Result<bool, Thrown> {
    let file = host.executing_file().unwrap_or_default();
    let line = host.executing_line();
    if let Some(log) = ini(b"mail.log").filter(|l| !l.is_empty()) {
        let mut logline = b"mail() on [".to_vec();
        logline.extend_from_slice(&file);
        logline.extend_from_slice(format!(":{line}]: To: ").as_bytes());
        logline.extend_from_slice(to);
        logline.extend_from_slice(b" -- Headers: ");
        logline.extend_from_slice(cstr(headers.unwrap_or_default()));
        logline.extend_from_slice(b" -- Subject: ");
        logline.extend_from_slice(subject);
        if headers.is_some() {
            for c in logline.iter_mut() {
                if matches!(*c, b'\r' | b'\n') {
                    *c = b' ';
                }
            }
        }
        if log == b"syslog" {
            crate::ext::standard::syslog::php_syslog_str(libc::LOG_NOTICE as i64, &logline);
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let mut entry = b"[".to_vec();
            entry.extend_from_slice(&crate::ext::date::php_format_date(
                b"d-M-Y H:i:s e",
                now,
                true,
            ));
            entry.extend_from_slice(b"] ");
            entry.extend_from_slice(&logline);
            entry.push(b'\n');
            log_to_file(host, &log, &entry)?;
        }
    }
    if let Some(t) = host.take_pending_throw() {
        return Err(t);
    }
    let mode = ini(b"mail.cr_lf_mode");
    let line_sep: &[u8] = match mode.as_deref() {
        Some(b"lf" | b"mixed" | b"os") => b"\n",
        _ if ini(b"mail.mixed_lf_and_crlf").is_some_and(|v| ini_parse_bool(&v)) => b"\n",
        _ => b"\r\n",
    };
    let mut hdr = headers.map(|h| cstr(h).to_vec());
    if ini(b"mail.add_x_header").is_some_and(|v| ini_parse_bool(&v)) {
        let base = file.rsplit(|&c| c == b'/').next().unwrap_or(&file).to_vec();
        let uid = crate::ext::standard::pageinfo::script_stat(host)
            .map_or(-1, |m| std::os::unix::fs::MetadataExt::uid(&m) as i64);
        let mut a = format!("X-PHP-Originating-Script: {uid}:").into_bytes();
        a.extend_from_slice(&base);
        if let Some(h) = hdr.as_deref().filter(|h| !h.is_empty()) {
            a.extend_from_slice(line_sep);
            a.extend_from_slice(h);
        }
        hdr = Some(a);
    }
    if hdr.as_deref().is_some_and(multiple_crlf) {
        crate::warn(
            host,
            "mail",
            "Multiple or malformed newlines found in additional_header",
        )?;
        return Ok(false);
    }
    let Some(path) = ini(b"sendmail_path") else {
        return Ok(false);
    };
    let mut cmd = path.clone();
    if let Some(extra) = extra_cmd {
        cmd.push(b' ');
        cmd.extend_from_slice(extra);
    }
    let mut body = Vec::new();
    for part in [
        b"To: ".as_slice(),
        to,
        line_sep,
        b"Subject: ",
        subject,
        line_sep,
    ] {
        body.extend_from_slice(part);
    }
    if let Some(h) = &hdr {
        body.extend_from_slice(h);
        body.extend_from_slice(line_sep);
    }
    body.extend_from_slice(line_sep);
    let message = cstr(message);
    if mode.as_deref() == Some(b"lf") {
        let mut i = 0;
        while i < message.len() {
            if message[i] == b'\r' && message.get(i + 1) == Some(&b'\n') {
                body.push(b'\n');
                i += 2;
            } else {
                body.push(message[i]);
                i += 1;
            }
        }
    } else {
        body.extend_from_slice(message);
    }
    body.extend_from_slice(line_sep);

    let could_not = |host: &mut dyn Host| {
        let m = format!("Could not execute mail delivery program '{}'", lossy(&path));
        crate::warn(host, "mail", &m)
    };
    let Some((r, w)) = make_pipe() else {
        could_not(host)?;
        return Ok(false);
    };
    let descs = [Desc {
        index: 0,
        childend: r,
        parentend: None,
    }];
    let pid = match spawn(SpawnCmd::Shell(&cmd), &descs, None, None) {
        Ok(pid) => pid,
        Err(_) => {
            could_not(host)?;
            return Ok(false);
        }
    };
    drop(descs);
    {
        // SAFETY: the fd is ours; the File closes it when done.
        let mut f = unsafe { std::fs::File::from_raw_fd(w.as_raw_fd()) };
        std::mem::forget(w);
        let _ = f.write_all(&body);
    }
    let (r, st) = waitpid_retry(pid, 0);
    if r == -1 {
        let e = std::io::Error::last_os_error();
        let m = format!(
            "Sendmail pclose failed {} ({})",
            e.raw_os_error().unwrap_or(0),
            crate::main::streams::strerror(e.raw_os_error().unwrap_or(0))
        );
        crate::warn(host, "mail", &m)?;
        return Ok(false);
    }
    if libc::WIFSIGNALED(st) {
        let sig = libc::WTERMSIG(st);
        // SAFETY: strsignal returns a static, NUL-terminated string.
        let name = unsafe { std::ffi::CStr::from_ptr(libc::strsignal(sig)) };
        let m = format!(
            "Sendmail killed by signal {sig} ({})",
            name.to_string_lossy()
        );
        crate::warn(host, "mail", &m)?;
        return Ok(false);
    }
    if !libc::WIFEXITED(st) {
        crate::warn(host, "mail", "Sendmail did not exit")?;
        return Ok(false);
    }
    let ret = libc::WEXITSTATUS(st);
    // EX_OK and EX_TEMPFAIL count as sent.
    if ret != 0 && ret != 75 {
        let m = format!("Sendmail exited with non-zero exit code {ret}");
        crate::warn(host, "mail", &m)?;
        return Ok(false);
    }
    Ok(true)
}

fn fn_mail(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let to = zpp_path(args, 0, "mail", "to")?;
    let subject = zpp_path(args, 1, "mail", "subject")?;
    let message = zpp_path(args, 2, "mail", "message")?;
    let headers = match args.get(3).map(deref) {
        None => None,
        Some(Value::Array(a)) => Some(build_headers(host, &a)?),
        Some(v) => {
            let s = conv::to_string_bytes(&v);
            if s.contains(&0) {
                return Err(crate::zend::zpp::value_error(
                    "mail",
                    4,
                    "additional_headers",
                    "must not contain any null bytes",
                ));
            }
            // php_trim(headers, NULL, 0, 2): trailing " \t\n\r\v\0" off.
            let mut n = s.len();
            while n > 0 && matches!(s[n - 1], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0) {
                n -= 1;
            }
            Some(s[..n].to_vec())
        }
    };
    let extra = match args.get(4) {
        Some(_) => Some(zpp_path(args, 4, "mail", "additional_params")?),
        None => None,
    };
    let to = if to.is_empty() { to } else { clean_field(&to) };
    let subject = if subject.is_empty() {
        subject
    } else {
        clean_field(&subject)
    };
    let extra = match ini(b"mail.force_extra_parameters") {
        Some(f) => Some(crate::ext::standard::exec::escape_shell_cmd(&f)),
        None => extra.map(|e| crate::ext::standard::exec::escape_shell_cmd(&e)),
    };
    let headers = headers.filter(|h| !h.is_empty());
    let ok = php_mail(
        host,
        cstr(&to),
        cstr(&subject),
        &message,
        headers.as_deref(),
        extra.as_deref(),
    )?;
    Ok(Value::Bool(ok))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("mail", fn_mail)]);
}
