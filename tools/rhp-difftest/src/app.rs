//! `rhp-difftest app`: differential web-request harness over the pinned
//! fixture apps (`tests/apps/apps.toml`).
//!
//! Each app has a request corpus (`tests/apps/<app>/requests.toml`). The
//! corpus runs through two CGI engines, one after the other, against the
//! **same absolute app path**: the reference `php-cgi -n -d opcache.enable=0`
//! and the candidate (`rhp --cgi`). Before each engine the app's writable
//! state (caches, logs, sessions, compiled views) is restored from one
//! pristine snapshot, so neither engine sees what the other wrote, and
//! absolute paths baked into caches are identical. Both engines get the same
//! scrubbed environment (`env -i` plus a fixed list and the CGI variables).
//!
//! Responses are parsed into status, ordered headers and body, then put
//! through *named* normalization rules (HTTP dates, cookie `expires=`, the
//! php version in `X-Powered-By`, Laravel's encrypted cookies decrypted with
//! the app's fixed `APP_KEY`, consistent alpha-renaming of session ids and
//! CSRF tokens, log timestamps); the report says which rules fired. The files
//! each engine wrote are diffed too, as the pseudo-request `@files`.
//!
//! `--check-known <file>` is a ratchet like `phpt`'s: listed request ids must
//! stay SAME.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use rhp_builtins::pcre::Pattern as Regex;

use crate::phpt::split_cgi;
use crate::{read_known, replace_bytes};

// ---------------------------------------------------------------------------
// manifest + corpus
// ---------------------------------------------------------------------------

/// One `[[app]]` entry of `apps.toml`, resolved to absolute paths.
struct App {
    name: String,
    root: PathBuf,
    docroot: PathBuf,
    /// Absolute path of the front controller (`SCRIPT_FILENAME`).
    front: PathBuf,
    /// `SCRIPT_NAME`: the front controller relative to the docroot.
    script_name: String,
}

/// How the pristine state is produced before the engines run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Warmup {
    /// Use the state `apps.py setup` left.
    None,
    /// Clear `warmup_clear`, run the whole corpus once through the
    /// reference CGI engine, drop `warmup_drop`, snapshot the result.
    Corpus,
}

struct Mask {
    name: String,
    re: Regex,
    replace: Vec<u8>,
    /// Where it applies: any of "headers", "body", "stderr", "files".
    targets: Vec<String>,
    /// The passes it applies in (`pass = [...]`); empty means every pass.
    passes: Vec<String>,
}

/// One run of the whole corpus under its own conditions. The corpus always
/// has the `default` pass; `[[pass]]` tables add more, e.g. one with
/// `APP_DEBUG=1` in the environment, or one that starts from an empty cache.
#[derive(Clone, Debug)]
struct Pass {
    name: String,
    /// Extra environment for every request of the pass (warm-up included).
    env: Vec<(String, String)>,
    warmup: Warmup,
    /// State dirs emptied right before the pristine snapshot.
    clear: Vec<String>,
    /// Only these request ids (with the flows they need); `None` = all.
    only: Option<BTreeSet<String>>,
}

impl Pass {
    /// The id a verdict has in reports and known-passing lists:
    /// `<pass>/<id>`, or the bare id in the default pass.
    fn key(&self, id: &str) -> String {
        if self.name == "default" {
            id.to_string()
        } else {
            format!("{}/{id}", self.name)
        }
    }

    /// The requests this pass runs, in corpus order: all, or those `only`
    /// names plus every request of their flows (a flow's later steps need
    /// its earlier ones).
    fn select<'r>(&self, requests: &'r [Request]) -> Vec<&'r Request> {
        let Some(only) = &self.only else {
            return requests.iter().collect();
        };
        let flows: BTreeSet<&str> = requests
            .iter()
            .filter(|r| only.contains(&r.id))
            .filter_map(|r| r.flow.as_deref())
            .collect();
        requests
            .iter()
            .filter(|r| {
                only.contains(&r.id) || r.flow.as_deref().is_some_and(|f| flows.contains(f))
            })
            .collect()
    }
}

struct Request {
    id: String,
    method: String,
    /// Path plus query string, as `REQUEST_URI`.
    uri: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    content_type: Option<String>,
    /// Requests sharing a flow share a cookie jar (per engine); a request
    /// without one starts with an empty jar.
    flow: Option<String>,
    /// The status the reference must answer with (`status = 404`); without
    /// it, any status below 500. See [`reference_failures`].
    status: Option<u16>,
    /// Per-pass overrides of `status` (`pass_status = { debug = 500 }`).
    pass_status: BTreeMap<String, u16>,
    /// `warmup = false`: left out of the warm-up run, for a request whose
    /// effect must not be in the pristine state (a comment posted twice
    /// is a duplicate).
    in_warmup: bool,
}

impl Request {
    /// The status the reference must answer with in `pass`.
    fn expected_status(&self, pass: &str) -> Option<u16> {
        self.pass_status.get(pass).copied().or(self.status)
    }
}

struct Corpus {
    state: Vec<String>,
    warmup_clear: Vec<String>,
    warmup_drop: Vec<String>,
    ini: Vec<String>,
    tokens: Vec<Regex>,
    masks: Vec<Mask>,
    /// `default` first, then the `[[pass]]` tables in order.
    passes: Vec<Pass>,
    /// Laravel's `APP_KEY` (decoded), when the app encrypts its cookies.
    laravel_key: Option<Vec<u8>>,
    requests: Vec<Request>,
}

fn str_of(t: &toml::Table, k: &str) -> Option<String> {
    t.get(k).and_then(|v| v.as_str()).map(String::from)
}

fn strs_of(t: &toml::Table, k: &str) -> Result<Vec<String>, String> {
    match t.get(k) {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(String::from)
                    .ok_or_else(|| format!("{k}: expected strings"))
            })
            .collect(),
        Some(_) => Err(format!("{k}: expected an array of strings")),
    }
}

fn regex(p: &str) -> Result<Regex, String> {
    Regex::new(p, 0).map_err(|e| format!("bad regex {p:?}: {e}"))
}

/// `KEY=value` from a dotenv file (no interpolation; quotes stripped).
fn dotenv_value(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.trim().split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn load_corpus(app: &App, path: &Path) -> Result<Corpus, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let t: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let warmup = match str_of(&t, "warmup").as_deref() {
        None | Some("none") => Warmup::None,
        Some("corpus") => Warmup::Corpus,
        Some(w) => return Err(format!("warmup: unknown mode {w:?} (none | corpus)")),
    };
    let tokens = strs_of(&t, "tokens")?
        .iter()
        .map(|p| regex(p))
        .collect::<Result<_, _>>()?;
    let mut masks = Vec::new();
    for m in t
        .get("mask")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let m = m.as_table().ok_or("mask: expected tables")?;
        masks.push(Mask {
            name: str_of(m, "name").ok_or("mask: needs a name")?,
            re: regex(&str_of(m, "pattern").ok_or("mask: needs a pattern")?)?,
            replace: str_of(m, "replace").unwrap_or_default().into_bytes(),
            targets: match strs_of(m, "in")? {
                v if v.is_empty() => vec!["body".into()],
                v => v,
            },
            passes: strs_of(m, "pass")?,
        });
    }
    let mut passes = vec![Pass {
        name: "default".into(),
        env: Vec::new(),
        warmup,
        clear: Vec::new(),
        only: None,
    }];
    for p in t
        .get("pass")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let p = p.as_table().ok_or("pass: expected tables")?;
        let name = str_of(p, "name").ok_or("pass: needs a name")?;
        let ok = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok || passes.iter().any(|q| q.name == name) {
            return Err(format!(
                "pass {name:?}: duplicate, reserved or not [a-z0-9-]+"
            ));
        }
        let mut env = Vec::new();
        if let Some(e) = p.get("env") {
            let e = e.as_table().ok_or("pass env: expected a table")?;
            for (k, v) in e {
                let v = v
                    .as_str()
                    .ok_or_else(|| format!("pass {name}: env {k} must be a string"))?;
                env.push((k.clone(), v.to_string()));
            }
        }
        let warmup = match str_of(p, "warmup").as_deref() {
            None => warmup,
            Some("none") => Warmup::None,
            Some("corpus") => Warmup::Corpus,
            Some(w) => return Err(format!("pass {name}: unknown warmup {w:?}")),
        };
        let only = match p.get("only") {
            None => None,
            Some(_) => Some(strs_of(p, "only")?.into_iter().collect()),
        };
        passes.push(Pass {
            name,
            env,
            warmup,
            clear: strs_of(p, "clear")?,
            only,
        });
    }
    let known = |n: &String| passes.iter().any(|p| &p.name == n);
    for m in &masks {
        if let Some(bad) = m.passes.iter().find(|n| !known(n)) {
            return Err(format!("mask {}: no pass {bad:?}", m.name));
        }
    }
    let laravel_key = match str_of(&t, "encrypted_cookies").as_deref() {
        None => None,
        Some("laravel") => {
            let raw = dotenv_value(&app.root.join(".env"), "APP_KEY")
                .ok_or("encrypted_cookies = \"laravel\" but no APP_KEY in .env")?;
            let key = match raw.strip_prefix("base64:") {
                Some(b) => b64(b).ok_or("APP_KEY: bad base64")?,
                None => raw.into_bytes(),
            };
            Some(key)
        }
        Some(k) => return Err(format!("encrypted_cookies: unknown kind {k:?}")),
    };
    let mut requests = Vec::new();
    let mut ids = BTreeSet::new();
    for r in t
        .get("request")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let r = r.as_table().ok_or("request: expected tables")?;
        let id = str_of(r, "id").ok_or("request: needs an id")?;
        if id.starts_with('@') || !ids.insert(id.clone()) {
            return Err(format!("request id {id:?}: duplicate or reserved"));
        }
        let headers = strs_of(r, "headers")?
            .iter()
            .map(|h| {
                h.split_once(':')
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .ok_or_else(|| format!("{id}: header {h:?} has no ':'"))
            })
            .collect::<Result<_, _>>()?;
        let mut pass_status = BTreeMap::new();
        if let Some(ps) = r.get("pass_status") {
            let ps = ps
                .as_table()
                .ok_or_else(|| format!("{id}: pass_status must be a table"))?;
            for (k, v) in ps {
                let n = v
                    .as_integer()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| format!("{id}: pass_status.{k} must be an HTTP status code"))?;
                pass_status.insert(k.clone(), n);
            }
        }
        requests.push(Request {
            pass_status,
            in_warmup: match r.get("warmup") {
                None => true,
                Some(v) => v
                    .as_bool()
                    .ok_or_else(|| format!("{id}: warmup must be true or false"))?,
            },
            method: str_of(r, "method").unwrap_or_else(|| "GET".into()),
            uri: str_of(r, "path").ok_or_else(|| format!("{id}: needs a path"))?,
            headers,
            body: str_of(r, "body").unwrap_or_default().into_bytes(),
            content_type: str_of(r, "content_type"),
            flow: str_of(r, "flow"),
            status: match r.get("status") {
                None => None,
                Some(v) => Some(
                    v.as_integer()
                        .and_then(|n| u16::try_from(n).ok())
                        .ok_or_else(|| format!("{id}: status must be an HTTP status code"))?,
                ),
            },
            id,
        });
    }
    for r in &requests {
        if let Some(bad) = r
            .pass_status
            .keys()
            .find(|n| !passes.iter().any(|p| &&p.name == n))
        {
            return Err(format!("{}: pass_status names no pass {bad:?}", r.id));
        }
    }
    Ok(Corpus {
        state: strs_of(&t, "state")?,
        warmup_clear: strs_of(&t, "warmup_clear")?,
        warmup_drop: strs_of(&t, "warmup_drop")?,
        ini: strs_of(&t, "ini")?,
        tokens,
        masks,
        passes,
        laravel_key,
        requests,
    })
}

/// The manifest to use: `--manifest`, else `tests/apps/apps.toml` above the
/// working directory, else the one in this source tree.
fn find_manifest(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    let mut dir = std::env::current_dir().ok();
    while let Some(d) = dir {
        let m = d.join("tests/apps/apps.toml");
        if m.is_file() {
            return Some(m);
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    let m = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/apps/apps.toml");
    m.is_file().then_some(m)
}

/// Resolve `target` (a manifest name, or an app directory) to its app.
fn resolve_app(target: &str, manifest: Option<&Path>) -> Result<App, String> {
    let as_dir = Path::new(target);
    let (manifest, name) = if as_dir.is_dir() && as_dir.join("requests.toml").is_file() {
        let dir = as_dir
            .canonicalize()
            .map_err(|e| format!("{target}: {e}"))?;
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let m = match manifest {
            Some(m) => m.to_path_buf(),
            None => dir.parent().unwrap_or(&dir).join("apps.toml"),
        };
        (m, name)
    } else {
        let m = find_manifest(manifest).ok_or("no tests/apps/apps.toml found (pass --manifest)")?;
        (m, target.to_string())
    };
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let t: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    let entries = t
        .get("app")
        .and_then(|v| v.as_array())
        .ok_or("apps.toml: no [[app]]")?;
    let entry = entries
        .iter()
        .filter_map(|e| e.as_table())
        .find(|e| str_of(e, "name").as_deref() == Some(name.as_str()))
        .ok_or_else(|| format!("app {name:?} is not in {}", manifest.display()))?;
    let root = manifest
        .parent()
        .unwrap_or(Path::new("."))
        .join(&name)
        .canonicalize()
        .map_err(|e| format!("app dir for {name}: {e}"))?;
    let docroot = root.join(str_of(entry, "docroot").unwrap_or_else(|| "public".into()));
    let front =
        root.join(str_of(entry, "front_controller").unwrap_or_else(|| "public/index.php".into()));
    let script_name = format!(
        "/{}",
        front
            .strip_prefix(&docroot)
            .map_err(|_| format!("{name}: front_controller is outside the docroot"))?
            .to_string_lossy()
    );
    Ok(App {
        name,
        root,
        docroot,
        front,
        script_name,
    })
}

// ---------------------------------------------------------------------------
// app state: snapshot / restore / written-file diff
// ---------------------------------------------------------------------------

/// The files (with mtimes) and directories under an app's state dirs.
#[derive(Default)]
struct Snapshot {
    dirs: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, (Vec<u8>, Option<SystemTime>)>,
}

fn take_snapshot(root: &Path, state: &[String]) -> Snapshot {
    fn walk(root: &Path, rel: &Path, snap: &mut Snapshot) {
        let p = root.join(rel);
        if p.is_file() {
            // A state entry can be a single file (`config/reference.php`).
            if let Ok(data) = fs::read(&p) {
                let mtime = fs::metadata(&p).ok().and_then(|m| m.modified().ok());
                snap.files.insert(rel.to_path_buf(), (data, mtime));
            }
            return;
        }
        let Ok(entries) = fs::read_dir(p) else {
            return;
        };
        snap.dirs.insert(rel.to_path_buf());
        for e in entries.flatten() {
            let sub = rel.join(e.file_name());
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(root, &sub, snap),
                Ok(t) if t.is_file() => {
                    if let Ok(data) = fs::read(root.join(&sub)) {
                        let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
                        snap.files.insert(sub, (data, mtime));
                    }
                }
                _ => {}
            }
        }
    }
    let mut snap = Snapshot::default();
    for s in state {
        walk(root, Path::new(s), &mut snap);
    }
    snap
}

/// Put the state dirs back exactly as `snap` has them (mtimes included:
/// Blade and Twig decide freshness by them).
fn restore_snapshot(root: &Path, state: &[String], snap: &Snapshot) -> std::io::Result<()> {
    for s in state {
        let p = root.join(s);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&p)?,
            Ok(_) => fs::remove_file(&p)?,
            Err(_) => {}
        }
    }
    for d in &snap.dirs {
        fs::create_dir_all(root.join(d))?;
    }
    for (rel, (data, mtime)) in &snap.files {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, data)?;
        if let Some(t) = mtime {
            let _ = fs::File::options()
                .write(true)
                .open(&p)
                .and_then(|f| f.set_modified(*t));
        }
    }
    Ok(())
}

/// Files an engine run created, changed (`Some(new content)`) or deleted
/// (`None`), relative to the pristine snapshot.
fn written_files(pristine: &Snapshot, after: &Snapshot) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    for (p, (data, _)) in &after.files {
        if pristine.files.get(p).map(|(d, _)| d) != Some(data) {
            out.insert(p.clone(), Some(data.clone()));
        }
    }
    for p in pristine.files.keys() {
        if !after.files.contains_key(p) {
            out.insert(p.clone(), None);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// running one request through a CGI engine
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Engine {
    label: &'static str,
    /// Program and leading args; the request goes in through the environment.
    cmd: Vec<String>,
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    stderr: Vec<u8>,
    /// Set when the engine did not produce a CGI response (spawn failure,
    /// timeout, signal, no header block).
    error: Option<String>,
}

/// The fixed part of every request's environment: nothing from the
/// harness's own environment leaks in except PATH and HOME.
fn base_env() -> Vec<(String, String)> {
    let mut env = vec![
        ("TZ".to_string(), "UTC".to_string()),
        ("LANG".to_string(), "C".to_string()),
    ];
    for k in ["PATH", "HOME"] {
        if let Ok(v) = std::env::var(k) {
            env.push((k.to_string(), v));
        }
    }
    env
}

/// The script a request runs and its `SCRIPT_NAME`: the `.php` file under
/// the docroot that the path names (`/wp-login.php`), as a web server's
/// `location ~ \.php$` does; any other path goes to the front controller.
fn script_for(app: &App, uri: &str) -> (PathBuf, String) {
    let path = uri.split_once('?').map_or(uri, |(p, _)| p);
    if path.ends_with(".php") && !path.split('/').any(|c| c == "..") {
        let file = app.docroot.join(path.trim_start_matches('/'));
        if file.is_file() {
            return (file, path.to_string());
        }
    }
    (app.front.clone(), app.script_name.clone())
}

/// The CGI/1.1 environment for one request.
fn cgi_env(
    app: &App,
    method: &str,
    req: &Request,
    body_len: usize,
    cookie: &str,
) -> Vec<(String, String)> {
    let query = req.uri.split_once('?').map_or("", |(_, q)| q);
    let mut env = base_env();
    let mut set = |k: &str, v: &str| env.push((k.to_string(), v.to_string()));
    set("REDIRECT_STATUS", "1");
    set("GATEWAY_INTERFACE", "CGI/1.1");
    set("SERVER_SOFTWARE", "rhp-difftest");
    set("SERVER_PROTOCOL", "HTTP/1.1");
    set("SERVER_NAME", "localhost");
    set("SERVER_PORT", "80");
    set("REMOTE_ADDR", "127.0.0.1");
    set("REMOTE_PORT", "50000");
    set("REQUEST_METHOD", method);
    set("REQUEST_URI", &req.uri);
    set("QUERY_STRING", query);
    set("DOCUMENT_ROOT", &app.docroot.to_string_lossy());
    let (script, script_name) = script_for(app, &req.uri);
    set("SCRIPT_FILENAME", &script.to_string_lossy());
    set("SCRIPT_NAME", &script_name);
    set("HTTP_HOST", "localhost");
    if body_len > 0 || method == "POST" || method == "PUT" || method == "PATCH" {
        let ct = req
            .content_type
            .as_deref()
            .unwrap_or("application/x-www-form-urlencoded");
        set("CONTENT_TYPE", ct);
        set("CONTENT_LENGTH", &body_len.to_string());
    }
    if !cookie.is_empty() {
        set("HTTP_COOKIE", cookie);
    }
    env
}

/// `Some-Header` → `HTTP_SOME_HEADER`.
fn header_var(name: &str) -> String {
    let mut v = String::from("HTTP_");
    v.extend(name.chars().map(|c| {
        if c == '-' {
            '_'
        } else {
            c.to_ascii_uppercase()
        }
    }));
    v
}

fn run_engine(
    engine: &Engine,
    cwd: &Path,
    env: &[(String, String)],
    stdin: &[u8],
    timeout: Duration,
) -> Response {
    let mut cmd = Command::new(&engine.cmd[0]);
    cmd.args(&engine.cmd[1..])
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Response {
                error: Some(format!("spawn {}: {e}", engine.cmd[0])),
                ..Response::default()
            }
        }
    };
    let input = stdin.to_vec();
    let mut pipe_in = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(p) = pipe_in.as_mut() {
            let _ = p.write_all(&input);
        }
    });
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out_t = reader(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err_t = reader(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => {
                let _ = child.kill();
                timed_out = true;
                break child.wait().ok();
            }
        }
    };
    let _ = writer.join();
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    let mut resp = parse_response(&stdout);
    resp.stderr = stderr;
    if timed_out {
        resp.error = Some("timeout".into());
    } else if let Some(sig) = status.and_then(|s| std::os::unix::process::ExitStatusExt::signal(&s))
    {
        resp.error = Some(format!("killed by signal {sig}"));
    } else if let Some(e) = &mut resp.error {
        // Say what the engine printed instead of a response.
        let said = [&resp.body, &resp.stderr].into_iter().find_map(|b| {
            let s = String::from_utf8_lossy(b);
            s.lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.chars().take(100).collect::<String>())
        });
        if let Some(said) = said {
            e.push_str(&format!(": {said}"));
        }
    }
    resp
}

/// Split CGI output into status, headers and body. The status comes from a
/// `Status:` header (default 200), which stays in the header list: it is
/// part of what the engine wrote.
fn parse_response(out: &[u8]) -> Response {
    let Some((head, body)) = split_cgi(out) else {
        return Response {
            body: out.to_vec(),
            error: Some("no CGI header block".into()),
            ..Response::default()
        };
    };
    let mut status = 200;
    let mut headers = Vec::new();
    for line in String::from_utf8_lossy(head).split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once(':').unwrap_or((line, ""));
        let v = v.trim_start();
        if k.eq_ignore_ascii_case("status") {
            status = v
                .split_whitespace()
                .next()
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
        }
        headers.push((k.to_string(), v.to_string()));
    }
    Response {
        status,
        headers,
        body: body.to_vec(),
        ..Response::default()
    }
}

// ---------------------------------------------------------------------------
// cookies
// ---------------------------------------------------------------------------

/// One engine's cookie jar for a flow: name → raw (still url-encoded) value.
#[derive(Default, Clone)]
struct Jar(BTreeMap<String, String>);

impl Jar {
    fn update(&mut self, headers: &[(String, String)]) {
        for (k, v) in headers {
            if !k.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let mut parts = v.split(';');
            let Some((name, value)) = parts.next().and_then(|nv| nv.split_once('=')) else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let expired = value == "deleted"
                || parts.any(|a| {
                    let a = a.trim().to_ascii_lowercase();
                    a.strip_prefix("max-age=")
                        .is_some_and(|n| n.parse::<i64>().is_ok_and(|n| n <= 0))
                });
            if expired {
                self.0.remove(name);
            } else {
                self.0.insert(name.to_string(), value.to_string());
            }
        }
    }

    fn header(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn url_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn b64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()
}

/// Decrypt one Laravel `Encrypter` payload (base64 JSON with `iv`, `value`,
/// `mac`): verify the HMAC-SHA256 over `iv . value`, then AES-256-CBC.
/// `Err` says why it isn't a valid payload under `key`.
fn laravel_decrypt(payload: &[u8], key: &[u8]) -> Result<Vec<u8>, &'static str> {
    use cbc::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
    use hmac::{digest::KeyInit, Mac};
    let json = b64(std::str::from_utf8(payload).map_err(|_| "not utf-8")?).ok_or("not base64")?;
    let json = String::from_utf8(json).map_err(|_| "not json")?;
    let field = |name: &str| -> Option<String> {
        let at = json.find(&format!("\"{name}\":\""))? + name.len() + 4;
        let end = json[at..].find('"')? + at;
        Some(json[at..end].replace("\\/", "/"))
    };
    let (iv, value, mac) = match (field("iv"), field("value"), field("mac")) {
        (Some(i), Some(v), Some(m)) => (i, v, m),
        _ => return Err("not an encrypter payload"),
    };
    let mut h = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).map_err(|_| "bad key")?;
    h.update(iv.as_bytes());
    h.update(value.as_bytes());
    let want: String = h
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if want != mac {
        return Err("bad mac");
    }
    let iv = b64(&iv).ok_or("bad iv")?;
    let mut buf = b64(&value).ok_or("bad value")?;
    cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &iv)
        .map_err(|_| "bad key/iv length")?
        .decrypt_padded::<Pkcs7>(&mut buf)
        .map(<[u8]>::to_vec)
        .map_err(|_| "bad padding")
}

/// A Laravel cookie's plaintext without its `CookieValuePrefix`
/// (`hash_hmac('sha1', name.'v2', key).'|'`).
fn strip_cookie_prefix(plain: &[u8]) -> &[u8] {
    match plain.iter().position(|&b| b == b'|') {
        Some(40) if plain[..40].iter().all(u8::is_ascii_hexdigit) => &plain[41..],
        _ => plain,
    }
}

/// Expand `{cookie:NAME}` (the jar's url-decoded value) and
/// `{cookie-plain:NAME}` (decrypted, prefix stripped) in `s`.
fn expand(s: &str, jar: &Jar, key: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(at) = rest.find('{') {
        out.extend_from_slice(&rest.as_bytes()[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find('}') else {
            out.extend_from_slice(tail.as_bytes());
            return Ok(out);
        };
        let inner = &tail[1..end];
        let value = if let Some(name) = inner.strip_prefix("cookie:") {
            let raw = jar
                .0
                .get(name)
                .ok_or_else(|| format!("no cookie {name:?} in the jar"))?;
            url_decode(raw)
        } else if let Some(name) = inner.strip_prefix("cookie-plain:") {
            let raw = jar
                .0
                .get(name)
                .ok_or_else(|| format!("no cookie {name:?} in the jar"))?;
            let key = key.ok_or("{cookie-plain:} needs encrypted_cookies")?;
            let plain = laravel_decrypt(&url_decode(raw), key)
                .map_err(|e| format!("cookie {name:?}: {e}"))?;
            strip_cookie_prefix(&plain).to_vec()
        } else {
            tail.as_bytes()[..=end].to_vec()
        };
        out.extend_from_slice(&value);
        rest = &tail[end + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    Ok(out)
}

// ---------------------------------------------------------------------------
// normalization
// ---------------------------------------------------------------------------

/// Replace every match of `re` with `rep`, where `$1`..`$9` in `rep` are
/// capture groups.
fn regex_replace(re: &Regex, hay: &[u8], rep: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut last = 0;
    for caps in re.captures_all(hay) {
        let Some((start, end)) = caps[0] else {
            continue;
        };
        out.extend_from_slice(&hay[last..start]);
        let mut i = 0;
        while i < rep.len() {
            if rep[i] == b'$' && i + 1 < rep.len() && rep[i + 1].is_ascii_digit() {
                let n = (rep[i + 1] - b'0') as usize;
                if let Some((s, e)) = caps.get(n).copied().flatten() {
                    out.extend_from_slice(&hay[s..e]);
                }
                i += 2;
            } else {
                out.push(rep[i]);
                i += 1;
            }
        }
        last = end;
    }
    out.extend_from_slice(&hay[last..]);
    out
}

/// Built-in timestamp masks for logs and stderr: Monolog/Symfony ISO 8601
/// (`[2026-11-01T12:00:00.123456+00:00]`) and Laravel's `[2026-11-01 12:00:00]`.
fn log_timestamp_masks() -> Vec<Regex> {
    [
        r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:[+-]\d\d:\d\d|Z)",
        r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d(?:\.\d+)?",
    ]
    .iter()
    .map(|p| regex(p).expect("builtin regex"))
    .collect()
}

/// Normalizes one engine's run. Token numbering is by first appearance
/// within the run, so equal tokens stay equal across headers, bodies,
/// later requests and written files, and both engines number alike when
/// they behave alike.
struct Normalizer<'a> {
    corpus: &'a Corpus,
    /// The pass being normalized (masks can be limited to passes).
    pass: &'a str,
    /// `(engine's PHP_VERSION, the candidate's)` for `X-Powered-By`.
    version: Option<(String, String)>,
    tokens: Vec<(Vec<u8>, String)>,
    timestamps: Vec<Regex>,
    fired: BTreeSet<String>,
}

impl<'a> Normalizer<'a> {
    fn new(corpus: &'a Corpus, pass: &'a str, version: Option<(String, String)>) -> Self {
        Normalizer {
            corpus,
            pass,
            version,
            tokens: Vec::new(),
            timestamps: log_timestamp_masks(),
            fired: BTreeSet::new(),
        }
    }

    fn register(&mut self, tok: &[u8]) {
        // Too short to be a random token: renaming it would rewrite text.
        if tok.len() < 8 || self.tokens.iter().any(|(t, _)| t == tok) {
            return;
        }
        let name = format!("<TOK{}>", self.tokens.len() + 1);
        self.tokens.push((tok.to_vec(), name));
    }

    fn rename(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        // Longest first, so a token containing another is replaced whole.
        let mut order: Vec<&(Vec<u8>, String)> = self.tokens.iter().collect();
        order.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        for (t, name) in order {
            let next = replace_bytes(&out, t, name.as_bytes());
            if next != out {
                self.fired.insert("token-rename".into());
                out = next;
            }
        }
        out
    }

    fn masks(&mut self, target: &str, data: Vec<u8>) -> Vec<u8> {
        let mut out = data;
        for m in &self.corpus.masks {
            let in_pass = m.passes.is_empty() || m.passes.iter().any(|p| p == self.pass);
            if in_pass && m.targets.iter().any(|t| t == target) {
                let next = regex_replace(&m.re, &out, &m.replace);
                if next != out {
                    self.fired.insert(format!("mask:{}", m.name));
                    out = next;
                }
            }
        }
        out
    }

    fn timestamps(&mut self, data: Vec<u8>) -> Vec<u8> {
        let mut out = data;
        for re in &self.timestamps {
            let next = regex_replace(re, &out, b"<TIMESTAMP>");
            if next != out {
                self.fired.insert("log-timestamp".into());
                out = next;
            }
        }
        out
    }

    fn response(&mut self, r: &Response) -> Response {
        let mut r = r.clone();
        for (k, v) in &mut r.headers {
            let lk = k.to_ascii_lowercase();
            // `Date` is always the time of the response (xmlrpc.php writes
            // its own, with `+0000`); the others only when they are dates.
            let dated = match lk.as_str() {
                "date" => true,
                "last-modified" | "expires" => v.contains(" GMT"),
                _ => false,
            };
            if dated {
                *v = "<HTTPDATE>".into();
                self.fired.insert("date-header".into());
            }
            if lk == "x-powered-by" {
                if let Some((ours, theirs)) = &self.version {
                    let from = format!("PHP/{ours}");
                    if ours != theirs && v.contains(&from) {
                        *v = v.replace(&from, &format!("PHP/{theirs}"));
                        self.fired.insert("php-version".into());
                    }
                }
            }
            if lk == "set-cookie" {
                // `expires=` and a positive `Max-Age=` both count from the
                // time of the request; `Max-Age=0` (a deletion) stays.
                let re = regex(r"(?i)(;\s*expires=)[^;]*").unwrap();
                let next = regex_replace(&re, v.as_bytes(), b"$1<DATE>");
                let re = regex(r"(?i)(;\s*max-age=)[1-9][0-9]*").unwrap();
                let next =
                    String::from_utf8_lossy(&regex_replace(&re, &next, b"$1<SECS>")).into_owned();
                if next != *v {
                    self.fired.insert("cookie-expires".into());
                    *v = next;
                }
                if let Some(key) = &self.corpus.laravel_key {
                    if let Some(nv) = self.decrypt_cookie(v, key) {
                        *v = nv;
                    }
                }
            }
        }
        // Collect tokens from headers then body, then rename everywhere.
        let head: Vec<u8> = r
            .headers
            .iter()
            .flat_map(|(k, v)| format!("{k}: {v}\n").into_bytes())
            .collect();
        self.collect(&head);
        self.collect(&r.body);
        for (_, v) in &mut r.headers {
            let nv = self.rename(v.as_bytes());
            let nv = self.masks("headers", nv);
            *v = String::from_utf8_lossy(&nv).into_owned();
        }
        let body = self.rename(&r.body);
        r.body = self.masks("body", body);
        let stderr = self.rename(&r.stderr);
        let stderr = self.timestamps(stderr);
        r.stderr = self.masks("stderr", stderr);
        r
    }

    /// `name=<encrypted>; attrs` → `name=decrypted(<plaintext>); attrs`, with
    /// the plaintext's value registered as a token.
    fn decrypt_cookie(&mut self, header: &str, key: &[u8]) -> Option<String> {
        let (nv, attrs) = header.split_once(';').unwrap_or((header, ""));
        let (name, value) = nv.split_once('=')?;
        let raw = url_decode(value);
        let shown = match laravel_decrypt(&raw, key) {
            Ok(plain) => {
                let v = strip_cookie_prefix(&plain).to_vec();
                let prefix = plain[..plain.len() - v.len()].to_vec();
                self.register(&v);
                let mut s = String::from_utf8_lossy(&prefix).into_owned();
                s.push_str(&String::from_utf8_lossy(&v));
                format!("decrypted({s})")
            }
            Err("not base64" | "not json" | "not utf-8" | "not an encrypter payload") => {
                return None
            }
            Err(e) => format!("undecryptable({e})"),
        };
        self.fired.insert("laravel-cookie".into());
        let attrs = if attrs.is_empty() {
            String::new()
        } else {
            format!(";{attrs}")
        };
        Some(format!("{name}={shown}{attrs}"))
    }

    /// Register every corpus `tokens` capture in `hay`.
    fn collect(&mut self, hay: &[u8]) {
        let found: Vec<Vec<u8>> = self
            .corpus
            .tokens
            .iter()
            .flat_map(|re| {
                re.captures_all(hay)
                    .into_iter()
                    .filter_map(|c| c.get(1).copied().flatten().map(|(s, e)| hay[s..e].to_vec()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for t in found {
            self.register(&t);
        }
    }

    /// Every written file, taken in the order of their renamed paths: a
    /// session file is named by its random id, so the raw order would
    /// number the tokens first seen inside them differently per engine.
    fn files(
        &mut self,
        written: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    ) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut order: Vec<(Vec<u8>, &PathBuf)> = written
            .keys()
            .map(|p| (self.rename(p.to_string_lossy().as_bytes()), p))
            .collect();
        order.sort();
        order
            .into_iter()
            .map(|(_, p)| self.file(p, &written[p]))
            .collect()
    }

    fn file(&mut self, path: &Path, data: &Option<Vec<u8>>) -> (String, Option<Vec<u8>>) {
        if let Some(d) = data {
            self.collect(d);
        }
        let p = self.rename(path.to_string_lossy().as_bytes());
        let p = String::from_utf8_lossy(&p).into_owned();
        let data = data.as_ref().map(|d| {
            let d = match sqlite_dump(d) {
                Some(dump) => {
                    self.fired.insert("sqlite-dump".into());
                    dump
                }
                None => d.clone(),
            };
            let d = self.rename(&d);
            let d = self.timestamps(d);
            self.masks("files", d)
        });
        (p, data)
    }
}

const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// A SQLite database file as `sqlite3 .dump` prints it: its schema and
/// rows as SQL text, without the page layout, free lists and change
/// counter, which differ between any two runs. `None` when `data` is not a
/// SQLite database; the dump's own error text when it can't be dumped.
fn sqlite_dump(data: &[u8]) -> Option<Vec<u8>> {
    if !data.starts_with(SQLITE_MAGIC) {
        return None;
    }
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("rhp-app-dump-{}-{n}.sqlite", std::process::id()));
    let out = fs::write(&tmp, data).and_then(|()| {
        Command::new("sqlite3")
            .arg("-readonly")
            .arg(&tmp)
            .arg(".dump")
            .stdin(Stdio::null())
            .output()
    });
    let _ = fs::remove_file(&tmp);
    Some(match out {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => format!(
            "<sqlite3 .dump failed: {}>\n",
            String::from_utf8_lossy(&o.stderr).trim()
        )
        .into_bytes(),
        Err(e) => format!("<sqlite3 .dump: {e}>\n").into_bytes(),
    })
}

// ---------------------------------------------------------------------------
// a whole engine run
// ---------------------------------------------------------------------------

struct EngineRun {
    /// Raw responses, in corpus order (`Err` = the request couldn't be sent).
    responses: Vec<Result<Response, String>>,
    written: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

type Env = Vec<(String, String)>;

/// One request's environment and stdin, placeholders expanded from `jar`.
fn build_request(
    app: &App,
    req: &Request,
    jar: &Jar,
    key: Option<&[u8]>,
    pass_env: &[(String, String)],
) -> Result<(Env, Vec<u8>), String> {
    let body = expand(&String::from_utf8_lossy(&req.body), jar, key)?;
    let mut env = cgi_env(app, &req.method, req, body.len(), &jar.header());
    env.extend(pass_env.iter().cloned());
    for (k, v) in &req.headers {
        let v = expand(v, jar, key)?;
        env.push((header_var(k), String::from_utf8_lossy(&v).into_owned()));
    }
    Ok((env, body))
}

fn run_corpus(
    app: &App,
    corpus: &Corpus,
    pass: &Pass,
    engine: &Engine,
    warmup: bool,
    timeout: Duration,
    verbose: bool,
) -> Vec<Result<Response, String>> {
    let mut jars: HashMap<String, Jar> = HashMap::new();
    let mut out = Vec::new();
    for req in pass.select(&corpus.requests) {
        if warmup && !req.in_warmup {
            continue;
        }
        let mut jar = req
            .flow
            .as_ref()
            .and_then(|f| jars.get(f).cloned())
            .unwrap_or_default();
        let built = build_request(app, req, &jar, corpus.laravel_key.as_deref(), &pass.env);
        let resp = built.map(|(env, body)| {
            let script = script_for(app, &req.uri).0;
            let cwd = script.parent().unwrap_or(&app.root);
            let started = Instant::now();
            let r = run_engine(engine, cwd, &env, &body, timeout);
            if verbose {
                eprintln!(
                    "  [{}] {} {} {} -> {} ({:.0} ms)",
                    engine.label,
                    pass.key(&req.id),
                    req.method,
                    req.uri,
                    r.status,
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            r
        });
        if let Ok(r) = &resp {
            jar.update(&r.headers);
        }
        if let Some(f) = &req.flow {
            jars.insert(f.clone(), jar);
        }
        out.push(resp);
    }
    out
}

/// `PHP_VERSION` as a CGI engine reports it (one request to a scratch script).
fn engine_version(engine: &Engine, scratch: &Path) -> Option<String> {
    let script = scratch.join("version.php");
    fs::write(&script, "<?php echo PHP_VERSION;").ok()?;
    let mut env = base_env();
    for (k, v) in [
        ("REDIRECT_STATUS", "1"),
        ("REQUEST_METHOD", "GET"),
        ("SCRIPT_FILENAME", &*script.to_string_lossy()),
        ("SCRIPT_NAME", "/version.php"),
        ("REQUEST_URI", "/version.php"),
        ("QUERY_STRING", ""),
    ] {
        env.push((k.to_string(), v.to_string()));
    }
    let r = run_engine(engine, scratch, &env, b"", Duration::from_secs(30));
    let v = String::from_utf8_lossy(&r.body).trim().to_string();
    (r.error.is_none() && !v.is_empty() && v.len() < 40).then_some(v)
}

// ---------------------------------------------------------------------------
// comparison + report
// ---------------------------------------------------------------------------

/// A response as comparable text: status line, headers, blank, body, then
/// stderr (if any) after a marker line.
fn render(r: &Result<Response, String>) -> String {
    match r {
        Err(e) => format!("<request not sent: {e}>\n"),
        Ok(r) => {
            let mut s = String::new();
            if let Some(e) = &r.error {
                s.push_str(&format!("<engine error: {e}>\n"));
            }
            s.push_str(&format!("status {}\n", r.status));
            for (k, v) in &r.headers {
                s.push_str(&format!("{k}: {v}\n"));
            }
            s.push('\n');
            s.push_str(&String::from_utf8_lossy(&r.body));
            if !r.stderr.is_empty() {
                s.push_str("\n--- stderr ---\n");
                s.push_str(&String::from_utf8_lossy(&r.stderr));
            }
            s
        }
    }
}

/// Where two normalized responses first differ, in one line.
fn first_difference(a: &Result<Response, String>, b: &Result<Response, String>) -> String {
    let short = |s: &str| -> String {
        let s: String = s.chars().take(70).collect();
        format!("{s:?}")
    };
    match (a, b) {
        (Ok(a), Ok(b)) => {
            if a.error != b.error {
                return format!("engine error: {:?} vs {:?}", a.error, b.error);
            }
            if a.status != b.status {
                return format!("status {} vs {}", a.status, b.status);
            }
            if a.headers != b.headers {
                let n = a.headers.len().max(b.headers.len());
                for i in 0..n {
                    let (x, y) = (a.headers.get(i), b.headers.get(i));
                    if x != y {
                        let show = |h: Option<&(String, String)>| {
                            h.map_or("<none>".to_string(), |(k, v)| short(&format!("{k}: {v}")))
                        };
                        return format!("header #{}: {} vs {}", i + 1, show(x), show(y));
                    }
                }
            }
            for (what, x, y) in [("body", &a.body, &b.body), ("stderr", &a.stderr, &b.stderr)] {
                if x != y {
                    let (xs, ys) = (String::from_utf8_lossy(x), String::from_utf8_lossy(y));
                    let (mut xl, mut yl) = (xs.split('\n'), ys.split('\n'));
                    let mut line = 1;
                    loop {
                        match (xl.next(), yl.next()) {
                            (Some(p), Some(q)) if p == q => line += 1,
                            (p, q) => {
                                return format!(
                                    "{what} line {line}: {} vs {}",
                                    short(p.unwrap_or("<eof>")),
                                    short(q.unwrap_or("<eof>"))
                                )
                            }
                        }
                    }
                }
            }
            "identical".into()
        }
        (a, b) => format!(
            "{} vs {}",
            a.as_ref().err().map_or("sent".to_string(), |e| e.clone()),
            b.as_ref().err().map_or("sent".to_string(), |e| e.clone())
        ),
    }
}

/// Requests the reference php-cgi did not answer the way the corpus says it
/// does: not sent, no CGI response (spawn failure, timeout, startup noise),
/// or the wrong status (any 5xx when the request names none). Such a run
/// says nothing about the candidate: a broken php-cgi setup (a missing
/// extension, an unwritable cache) would otherwise just turn every request
/// DIFF, or worse, SAME when both engines print the same error page.
fn reference_failures(
    requests: &[&Request],
    pass: &str,
    responses: &[Result<Response, String>],
) -> Vec<String> {
    requests
        .iter()
        .zip(responses)
        .filter_map(|(req, r)| {
            let why = match r {
                Err(e) => format!("not sent: {e}"),
                Ok(r) => match (&r.error, req.expected_status(pass)) {
                    (Some(e), _) => e.clone(),
                    (None, Some(want)) if r.status != want => {
                        format!("status {} (the corpus expects {want})", r.status)
                    }
                    (None, None) if r.status >= 500 => format!("status {}", r.status),
                    _ => return None,
                },
            };
            Some(format!("{} {} {}: {why}", req.id, req.method, req.uri))
        })
        .collect()
}

struct Verdict {
    id: String,
    same: bool,
    detail: String,
    rules: BTreeSet<String>,
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const USAGE: &str = "\
usage: rhp-difftest app <app-name | app-dir>... [options]

Runs each app's request corpus (tests/apps/<app>/requests.toml) through the
reference php-cgi and a candidate CGI engine, one after the other on the same
app path, and compares the normalized responses and written files.

  --manifest <file>      apps manifest (default: tests/apps/apps.toml above the cwd)
  --php-cgi <bin>        reference (default: env RHP_PHP_CGI, else php-cgi next to php)
  --candidate <cmd>      candidate CGI command, split on whitespace (default:
                         `<rhp> --cgi`, rhp next to this binary or env RHP_BIN);
                         both engines get `-n -d opcache.enable=0` and the
                         corpus `ini` as `-d` args, the request via the env
  --timeout <secs>       per-request timeout (default: 60)
  --check-known <file>   exit 1 if a listed request id is not SAME (one app only)
  --write-known <file>   write the SAME request ids (one app only)
  --out <dir>            write each engine's normalized responses + a diff summary
  --pass <name>          run only this pass of the corpus (repeatable; default:
                         every pass: `default` and the corpus `[[pass]]` tables)
  --verbose              log every request as it runs
";

struct Opts {
    targets: Vec<String>,
    manifest: Option<PathBuf>,
    php_cgi: Option<PathBuf>,
    candidate: Option<String>,
    timeout: u64,
    check_known: Option<PathBuf>,
    write_known: Option<PathBuf>,
    out: Option<PathBuf>,
    /// `--pass`: run only these passes (default: all).
    passes: Vec<String>,
    verbose: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        targets: Vec::new(),
        manifest: None,
        php_cgi: None,
        candidate: std::env::var("RHP_CGI_CANDIDATE").ok(),
        timeout: 60,
        check_known: None,
        write_known: None,
        out: None,
        passes: Vec::new(),
        verbose: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a.as_str() {
            "--manifest" => o.manifest = Some(val()?.into()),
            "--php-cgi" => o.php_cgi = Some(val()?.into()),
            "--candidate" => o.candidate = Some(val()?),
            "--timeout" => {
                o.timeout = val()?
                    .parse()
                    .map_err(|_| "--timeout needs seconds".to_string())?
            }
            "--check-known" => o.check_known = Some(val()?.into()),
            "--write-known" => o.write_known = Some(val()?.into()),
            "--out" => o.out = Some(val()?.into()),
            "--pass" => o.passes.push(val()?),
            "--verbose" | "-v" => o.verbose = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown option {s:?}")),
            s => o.targets.push(s.to_string()),
        }
    }
    if o.targets.is_empty() {
        return Err("name at least one app".into());
    }
    if (o.check_known.is_some() || o.write_known.is_some()) && o.targets.len() > 1 {
        return Err("--check-known/--write-known take one app".into());
    }
    Ok(o)
}

/// `rhp-difftest app ...` entry point.
pub fn main(args: &[String]) -> ExitCode {
    let o = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("rhp-difftest app: {e}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(php_cgi) = crate::phpt::find_php_cgi(o.php_cgi.as_deref()).filter(|p| p.is_file())
    else {
        eprintln!("rhp-difftest app: no php-cgi found (pass --php-cgi or set RHP_PHP_CGI)");
        return ExitCode::from(2);
    };
    let candidate: Vec<String> = match &o.candidate {
        Some(c) => c.split_whitespace().map(String::from).collect(),
        None => {
            let rhp = std::env::var_os("RHP_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join("rhp")))
                        .unwrap_or_else(|| PathBuf::from("rhp"))
                });
            vec![rhp.to_string_lossy().into_owned(), "--cgi".into()]
        }
    };
    if candidate.is_empty() {
        eprintln!("rhp-difftest app: empty --candidate");
        return ExitCode::from(2);
    }
    let mut code = ExitCode::SUCCESS;
    for target in &o.targets {
        match run_app(&o, target, &php_cgi, &candidate) {
            Ok(true) => {}
            Ok(false) => code = ExitCode::FAILURE,
            Err(e) => {
                eprintln!("rhp-difftest app {target}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    code
}

/// Run one app; `Ok(false)` when the ratchet failed.
fn run_app(o: &Opts, target: &str, php_cgi: &Path, candidate: &[String]) -> Result<bool, String> {
    let app = resolve_app(target, o.manifest.as_deref())?;
    let corpus = load_corpus(&app, &app.root.join("requests.toml"))?;
    let engine_args = |prefix: Vec<String>| -> Vec<String> {
        let mut cmd = prefix;
        cmd.extend(["-n", "-d", "opcache.enable=0"].map(String::from));
        for kv in &corpus.ini {
            cmd.push("-d".into());
            cmd.push(kv.clone());
        }
        cmd
    };
    let reference = Engine {
        label: "php-cgi",
        cmd: engine_args(vec![php_cgi.to_string_lossy().into_owned()]),
    };
    let cand = Engine {
        label: "candidate",
        cmd: engine_args(candidate.to_vec()),
    };
    let timeout = Duration::from_secs(o.timeout);
    let scratch = std::env::temp_dir().join(format!("rhp-app-{}-{}", app.name, std::process::id()));
    fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let (ref_v, cand_v) = (
        engine_version(&reference, &scratch),
        engine_version(&cand, &scratch),
    );
    println!(
        "app {}: reference `{}` (PHP {}) vs candidate `{}` (PHP {}), {} requests, passes: {}",
        app.name,
        reference.cmd.join(" "),
        ref_v.as_deref().unwrap_or("?"),
        cand.cmd.join(" "),
        cand_v.as_deref().unwrap_or("?"),
        corpus.requests.len(),
        corpus
            .passes
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let passes: Vec<&Pass> = corpus
        .passes
        .iter()
        .filter(|p| o.passes.is_empty() || o.passes.contains(&p.name))
        .collect();
    if let Some(bad) = o
        .passes
        .iter()
        .find(|n| !corpus.passes.iter().any(|p| &&p.name == n))
    {
        return Err(format!("no pass {bad:?} in the corpus"));
    }
    let version_pair = match (&ref_v, &cand_v) {
        (Some(r), Some(c)) => Some((r.clone(), c.clone())),
        _ => None,
    };

    // The app as found, put back at the end whatever happens in between.
    let original = take_snapshot(&app.root, &corpus.state);
    let ctx = PassCtx {
        o,
        app: &app,
        corpus: &corpus,
        reference: &reference,
        cand: &cand,
        timeout,
        version_pair,
        original: &original,
    };
    let mut results = Vec::new();
    let mut failure = None;
    for pass in &passes {
        if passes.len() > 1 {
            println!("pass {}:", pass.name);
        }
        match run_pass(&ctx, pass) {
            Ok(r) => results.push(r),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    let restored = restore_snapshot(&app.root, &corpus.state, &original);
    let _ = fs::remove_dir_all(&scratch);
    if let Some(e) = failure {
        return Err(e);
    }
    restored.map_err(|e| format!("restoring the app's original state: {e}"))?;

    let verdicts: Vec<&Verdict> = results.iter().flat_map(|r| &r.verdicts).collect();
    let same = verdicts.iter().filter(|v| v.same).count();
    println!("{}: {same} same, {} diff", app.name, verdicts.len() - same);

    let broken: Vec<&String> = results.iter().flat_map(|r| &r.broken).collect();
    if !broken.is_empty() {
        return Err(format!(
            "the reference php-cgi did not answer {} request(s) as the corpus expects \
             (a setup problem, not an rhp one; see --out/-v):\n  {}",
            broken.len(),
            broken
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        ));
    }
    if let Some(p) = &o.write_known {
        let mut s = format!(
            "# {} requests rhp answers like php-cgi (rhp-difftest app --write-known).\n\
             # CI fails if one stops matching: fix the regression, or remove the line.\n",
            app.name
        );
        for v in verdicts.iter().filter(|v| v.same) {
            s.push_str(&v.id);
            s.push('\n');
        }
        fs::write(p, s).map_err(|e| format!("{}: {e}", p.display()))?;
        println!("wrote {same} request id(s) to {}", p.display());
    }
    if let Some(p) = &o.check_known {
        let known = read_known(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let by_id: HashMap<&str, &Verdict> = verdicts.iter().map(|v| (v.id.as_str(), *v)).collect();
        // A key of a pass that was not run (`--pass`) is not checked.
        let ran = |k: &str| {
            let pass = k.split_once('/').map_or("default", |(p, _)| p);
            let pass = if corpus.passes.iter().any(|p| p.name == pass) {
                pass
            } else {
                "default"
            };
            passes.iter().any(|p| p.name == pass)
        };
        let mut failed = Vec::new();
        for k in known.iter().filter(|k| ran(k)) {
            match by_id.get(k.as_str()) {
                Some(v) if v.same => {}
                Some(v) => failed.push(format!("{k}: {}", v.detail)),
                None => failed.push(format!("{k}: no such request in the corpus")),
            }
        }
        let newly: Vec<&str> = verdicts
            .iter()
            .filter(|v| v.same && !known.contains(&v.id))
            .map(|v| v.id.as_str())
            .collect();
        if !newly.is_empty() {
            println!("newly SAME (add to {}): {}", p.display(), newly.join(" "));
        }
        if !failed.is_empty() {
            println!("{} known request(s) regressed:", failed.len());
            for f in &failed {
                println!("  {f}");
            }
            return Ok(false);
        }
        println!("all {} known request(s) still SAME", known.len());
    }
    Ok(true)
}

/// What every pass of one app shares.
struct PassCtx<'a> {
    o: &'a Opts,
    app: &'a App,
    corpus: &'a Corpus,
    reference: &'a Engine,
    cand: &'a Engine,
    timeout: Duration,
    version_pair: Option<(String, String)>,
    /// The state as the run found it; each pass starts from it.
    original: &'a Snapshot,
}

struct PassResult {
    /// Keyed `<pass>/<id>` outside the default pass, `@files` last.
    verdicts: Vec<Verdict>,
    /// The requests php-cgi did not answer as the corpus expects.
    broken: Vec<String>,
}

/// Run one pass: its pristine state, both engines, the comparison, the
/// printed report and `--out`.
fn run_pass(c: &PassCtx, pass: &Pass) -> Result<PassResult, String> {
    let (app, corpus) = (c.app, c.corpus);
    let io = |e: std::io::Error| format!("restoring app state: {e}");
    restore_snapshot(&app.root, &corpus.state, c.original).map_err(io)?;
    if pass.warmup == Warmup::Corpus {
        for d in &corpus.warmup_clear {
            let p = app.root.join(d);
            if p.is_dir() {
                fs::remove_dir_all(&p).map_err(io)?;
                fs::create_dir_all(&p).map_err(io)?;
            }
        }
        let warm = run_corpus(app, corpus, pass, c.reference, true, c.timeout, c.o.verbose);
        let failed = warm
            .iter()
            .filter(|r| !matches!(r, Ok(r) if r.error.is_none()))
            .count();
        if failed > 0 {
            eprintln!("warning: {failed} warm-up request(s) failed under php-cgi");
        }
        for d in &corpus.warmup_drop {
            let p = app.root.join(d);
            if p.is_dir() {
                fs::remove_dir_all(&p).map_err(io)?;
            }
        }
    }
    for d in &pass.clear {
        let p = app.root.join(d);
        if p.is_dir() {
            fs::remove_dir_all(&p).map_err(io)?;
            fs::create_dir_all(&p).map_err(io)?;
        }
    }
    let pristine = take_snapshot(&app.root, &corpus.state);
    let mut runs = Vec::new();
    for engine in [c.reference, c.cand] {
        restore_snapshot(&app.root, &corpus.state, &pristine).map_err(io)?;
        let responses = run_corpus(app, corpus, pass, engine, false, c.timeout, c.o.verbose);
        let after = take_snapshot(&app.root, &corpus.state);
        runs.push(EngineRun {
            responses,
            written: written_files(&pristine, &after),
        });
    }
    let requests = pass.select(&corpus.requests);
    let wrote_sqlite = runs
        .iter()
        .flat_map(|r| r.written.values().flatten())
        .any(|d| d.starts_with(SQLITE_MAGIC));
    if wrote_sqlite && Command::new("sqlite3").arg("-version").output().is_err() {
        return Err("a SQLite database was written: comparing it needs the sqlite3 command".into());
    }

    // Normalize each run with its own token numbering.
    let mut norm_ref = Normalizer::new(corpus, &pass.name, c.version_pair.clone());
    let mut norm_cand = Normalizer::new(corpus, &pass.name, None);
    let mut verdicts = Vec::new();
    let mut rendered: [Vec<(String, String)>; 2] = [Vec::new(), Vec::new()];
    for (i, req) in requests.iter().enumerate() {
        norm_ref.fired.clear();
        norm_cand.fired.clear();
        let a = runs[0].responses[i]
            .as_ref()
            .map(|r| norm_ref.response(r))
            .map_err(Clone::clone);
        let b = runs[1].responses[i]
            .as_ref()
            .map(|r| norm_cand.response(r))
            .map_err(Clone::clone);
        let same = a == b && matches!(&a, Ok(r) if r.error.is_none());
        let detail = if same {
            match &a {
                Ok(r) => format!("{} {} -> {}", req.method, req.uri, r.status),
                Err(_) => String::new(),
            }
        } else {
            format!("{} {}: {}", req.method, req.uri, first_difference(&a, &b))
        };
        rendered[0].push((req.id.clone(), render(&a)));
        rendered[1].push((req.id.clone(), render(&b)));
        verdicts.push(Verdict {
            id: pass.key(&req.id),
            same,
            detail,
            rules: norm_ref.fired.union(&norm_cand.fired).cloned().collect(),
        });
    }
    // Written files, normalized with the same token maps.
    norm_ref.fired.clear();
    norm_cand.fired.clear();
    let files_a = norm_ref.files(&runs[0].written);
    let files_b = norm_cand.files(&runs[1].written);
    let files_detail = {
        let names: BTreeSet<&String> = files_a.keys().chain(files_b.keys()).collect();
        let differing: Vec<String> = names
            .iter()
            .filter(|n| files_a.get(**n) != files_b.get(**n))
            .map(|n| {
                let state = |m: &BTreeMap<String, Option<Vec<u8>>>| match m.get(*n) {
                    None => "absent",
                    Some(None) => "deleted",
                    Some(Some(_)) => "written",
                };
                format!("{n} ({} vs {})", state(&files_a), state(&files_b))
            })
            .collect();
        if differing.is_empty() {
            (
                true,
                format!("{} file(s) written, all identical", files_a.len()),
            )
        } else {
            (
                false,
                format!(
                    "{} of {} file(s) differ: {}",
                    differing.len(),
                    names.len(),
                    differing
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        }
    };
    verdicts.push(Verdict {
        id: pass.key("@files"),
        same: files_detail.0,
        detail: files_detail.1,
        rules: norm_ref.fired.union(&norm_cand.fired).cloned().collect(),
    });

    if let Some(dir) = &c.o.out {
        let mut base = dir.join(&app.name);
        if pass.name != "default" {
            base = base.join(&pass.name);
        }
        for (k, label) in ["php-cgi", "candidate"].iter().enumerate() {
            let d = base.join(label);
            let _ = fs::create_dir_all(d.join("files"));
            for (id, text) in &rendered[k] {
                let _ = fs::write(d.join(format!("{id}.txt")), text);
            }
            let files = if k == 0 { &files_a } else { &files_b };
            for (n, data) in files {
                let flat = n.replace('/', "__");
                let _ = fs::write(
                    d.join("files").join(flat),
                    data.as_deref().unwrap_or(b"<deleted>"),
                );
            }
        }
    }

    let width = verdicts.iter().map(|v| v.id.len()).max().unwrap_or(0);
    for v in &verdicts {
        let rules = if v.rules.is_empty() {
            String::new()
        } else {
            format!(
                "  [{}]",
                v.rules.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        println!(
            "  {}  {:width$}  {}{rules}",
            if v.same { "SAME" } else { "DIFF" },
            v.id,
            v.detail
        );
    }
    let broken = reference_failures(&requests, &pass.name, &runs[0].responses)
        .into_iter()
        .map(|b| {
            if pass.name == "default" {
                b
            } else {
                format!("{}/{b}", pass.name)
            }
        })
        .collect();
    Ok(PassResult { verdicts, broken })
}

#[cfg(test)]
mod tests;
