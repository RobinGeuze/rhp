//! php-cli's do_cli modes that run in the request in place of a script
//! (sapi/cli/php_cli.c): the shell (`-a`), `-B`/`-R`/`-F`/`-E`, the
//! reflection modes (`--rf`, `--rc`, `--re`, `--rz`), `--ri`, `--ini`
//! and `-m`.

use rhp_builtins as builtins;
use rhp_values::{Thrown, Value};

use crate::Vm;

/// What runs in place of the primary script.
#[derive(Clone, Debug)]
pub enum CliMode {
    /// `-a`: the readline shell.
    Shell,
    /// `-B`/`-R`/`-F`/`-E` (PHP_CLI_MODE_PROCESS_STDIN): `run` (or the
    /// file `file`) for each line of standard input, `begin` before and
    /// `end` after.
    ProcessStdin {
        /// `-B` code.
        begin: Option<Vec<u8>>,
        /// `-R` code.
        run: Option<Vec<u8>>,
        /// `-F` file.
        file: Option<Vec<u8>>,
        /// `-E` code.
        end: Option<Vec<u8>>,
    },
    /// `--rf`/`--rc`/`--re`/`--rz`: `what` printed through the Reflection
    /// class `class` (`ReflectionMethod` for a `--rf` name with `::`).
    Reflection {
        /// The Reflection class.
        class: &'static str,
        /// The name asked for.
        what: Vec<u8>,
    },
    /// `--ri`: a module's phpinfo section.
    ExtInfo(Vec<u8>),
    /// `--ini`; the flag is `-n`.
    ShowIni(bool),
    /// `--ini=diff`.
    ShowIniDiff,
    /// `-m`.
    Modules,
}

impl Vm {
    /// php-cli: run `mode` instead of a script (php -a, -R, --rf, ...).
    pub fn set_cli_mode(&mut self, mode: CliMode) {
        self.cli_mode = Some(mode);
    }

    /// The `do_cli` switch for a mode that is not a script; returns the
    /// exit status.
    pub(crate) fn run_cli_mode(&mut self, mode: CliMode) -> Result<i32, Thrown> {
        match mode {
            CliMode::Shell => return Ok(builtins::readline_shell_run(self)),
            CliMode::ProcessStdin {
                begin,
                run,
                file,
                end,
            } => self.process_stdin(begin, run, file, end),
            CliMode::Reflection { class, what } => self.cli_reflection(class, what)?,
            CliMode::ExtInfo(name) => {
                let name = String::from_utf8_lossy(&name).into_owned();
                match builtins::ext_info_text(&name) {
                    Some(text) => self.write_output(&text),
                    None => {
                        let msg = format!("Extension '{name}' not present.\n");
                        self.write_output(msg.as_bytes());
                        self.sapi.exit_status = 1;
                    }
                }
            }
            CliMode::ShowIni(ignore) => self.write_output(&builtins::ini_config_text(ignore)),
            CliMode::ShowIniDiff => self.write_output(&builtins::ini_diff_text()),
            CliMode::Modules => self.write_output(&builtins::modules_text()),
        }
        Ok(self.sapi.exit_status)
    }

    /// PHP_CLI_MODE_PROCESS_STDIN: `$argn` / `$argi` set for each line of
    /// the STDIN stream (so `fgets(STDIN)` in the code takes lines from
    /// the same buffer), while the exit status stays 0.
    fn process_stdin(
        &mut self,
        begin: Option<Vec<u8>>,
        run: Option<Vec<u8>>,
        file: Option<Vec<u8>>,
        end: Option<Vec<u8>>,
    ) {
        if let Some(code) = begin {
            if self.cli_eval(&code, "Command line begin code") {
                return;
            }
        }
        let stdin = self.constants.get(&b"STDIN"[..]).cloned();
        let mut index = 0i64;
        while self.sapi.exit_status == 0 {
            let line = match &stdin {
                Some(s) => builtins::Host::stream_gets(self, s),
                None => None,
            };
            let Some(line) = line else {
                break;
            };
            // php: strlen, then the trailing CR/LF cut, but never below
            // one byte (an empty line gives "\0").
            let mut s = match line.iter().position(|&b| b == 0) {
                Some(n) => line[..n].to_vec(),
                None => line,
            };
            while s.last().is_some_and(|&b| b == b'\n' || b == b'\r') {
                s.pop();
            }
            if s.is_empty() {
                s.push(0);
            }
            index += 1;
            *self.global_cell(b"argn").borrow_mut() = Value::str(s);
            *self.global_cell(b"argi").borrow_mut() = Value::Int(index);
            let bailed = match (&run, &file) {
                (Some(code), _) => self.cli_eval(code, "Command line run code"),
                (None, Some(path)) => self.cli_execute_file(path.clone()),
                (None, None) => false,
            };
            if bailed {
                return;
            }
        }
        if let Some(code) = end {
            self.cli_eval(&code, "Command line end code");
        }
    }

    /// `zend_eval_string_ex(code, NULL, name, 1)` under do_cli's zend_try:
    /// an uncaught throwable or a parse error is reported without bailing
    /// out (exit status 255); true when a fatal error bailed out.
    fn cli_eval(&mut self, code: &[u8], name: &str) -> bool {
        self.eval_string_ex(code, name, (builtins::E_ERROR, "Fatal error"))
    }

    /// `php_execute_script` for `-F`'s file: auto_prepend_file, then the
    /// file, in the global scope; an uncaught throwable goes to the
    /// exception handler, else is reported (exit status 255). True: php
    /// bailed out.
    fn cli_execute_file(&mut self, path: Vec<u8>) -> bool {
        let mark = self.last_error.clone();
        let r = self.run_primary_file(path, false);
        let (bailed, append) = self.cli_script_result(r, &mark);
        if !append {
            return bailed;
        }
        // auto_append_file after the file, or after the handler took
        // its exception.
        let mark = self.last_error.clone();
        match self.run_auto_append() {
            Some(r) => self.cli_script_result(r, &mark).0,
            None => bailed,
        }
    }

    /// An -F file's (or its auto_append_file's) outcome: (php bailed
    /// out, auto_append_file runs next).
    fn cli_script_result(
        &mut self,
        r: Result<(), Thrown>,
        mark: &Option<(i64, Vec<u8>, Vec<u8>, i64)>,
    ) -> (bool, bool) {
        let mut bailed = false;
        let mut append = false;
        match r {
            Ok(()) => append = true,
            Err(Thrown::Exit(code)) => {
                self.sapi.exit_status = code;
                bailed = self.fatal_since(mark);
            }
            Err(t) if t.is_catchable() => {
                match self.top_level_throw(t) {
                    Ok(None) => append = true,
                    Ok(Some(code)) => self.sapi.exit_status = code,
                    Err(_) => self.sapi.exit_status = 255,
                }
                // The handler ran on the shutdown scope: the global scope
                // goes back to being the finished {main} for the next line.
                if !self.stack.is_empty() {
                    self.stack.truncate(1);
                    self.finished_main = self.stack.pop();
                }
                self.in_shutdown_scope = false;
            }
            Err(_) => self.sapi.exit_status = 255,
        }
        if let Some(Thrown::Exit(code)) = self.pending_throw.take() {
            self.sapi.exit_status = code;
            append = false;
        }
        (bailed, append && self.sapi.exit_status == 0)
    }

    /// The reflection modes: `new $class($what)` (or
    /// `ReflectionMethod::createFromMethodName`), printed with a newline;
    /// an exception prints "Exception: <message>" and sets status 1.
    fn cli_reflection(&mut self, class: &'static str, what: Vec<u8>) -> Result<(), Thrown> {
        let arg = Value::str(what.clone());
        let r = if class == "ReflectionFunction" && what.windows(2).any(|w| w == b"::") {
            let f = Value::str("ReflectionMethod::createFromMethodName");
            builtins::Host::call_callable(self, &f, &[arg])
        } else {
            builtins::Host::reflect_new_instance(self, class.as_bytes(), vec![arg], true)
        };
        let printed = r.and_then(|obj| match &obj {
            Value::Object(h) => self.call_to_string(h),
            _ => Ok(Value::str("")),
        });
        match printed {
            Ok(s) => {
                let mut out = rhp_values::conv::to_string_bytes(&s);
                out.push(b'\n');
                self.write_output(&out);
            }
            Err(t) if t.is_catchable() => {
                let t = self.materialize(t, None);
                let msg = match &t {
                    Thrown::Object(h) => h
                        .borrow()
                        .props()
                        .get(&rhp_values::PhpStr::from("message"))
                        .map(|v| rhp_values::conv::to_string_bytes(&rhp_values::ops::deref(v)))
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                let mut out = b"Exception: ".to_vec();
                out.extend_from_slice(&msg);
                out.push(b'\n');
                self.write_output(&out);
                self.sapi.exit_status = 1;
            }
            Err(t) => return Err(t),
        }
        Ok(())
    }
}
