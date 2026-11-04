//! A port of php's `php_getopt` (main/getopt.c).

use crate::cgi::OPTIONS;

/// One `php_getopt` result.
pub(crate) enum Opt {
    /// No more options (`php_optind` is the first operand).
    End,
    /// An invalid option (reported on stderr when asked to).
    Invalid,
    /// An option and its value.
    Char(u8, Option<String>),
}

/// A port of php's `php_getopt` (main/getopt.c): bundled short options
/// (`-qC`), `-dfoo` / `-d foo` / `-d=foo` values, `--long[=value]` names,
/// and its error messages.
pub(crate) struct PhpGetopt<'a> {
    args: &'a [String],
    pub(crate) optind: usize,
    optchr: usize,
    dash: bool,
}

impl<'a> PhpGetopt<'a> {
    pub(crate) fn new(args: &'a [String]) -> Self {
        PhpGetopt {
            args,
            optind: 1,
            optchr: 0,
            dash: false,
        }
    }

    /// Byte `i` of argument `ind`, 0 past its end (a C string).
    fn at(&self, ind: usize, i: usize) -> u8 {
        self.args
            .get(ind)
            .and_then(|a| a.as_bytes().get(i).copied())
            .unwrap_or(0)
    }

    fn error(&self, ind: usize, chr: usize, what: &str, show: bool) -> Opt {
        if show {
            let c = self.at(ind, chr) as char;
            let msg = match what {
                "colon" => ": in flags".to_string(),
                "nf" => format!("option not found {c}"),
                _ => format!("no argument for option {c}"),
            };
            eprintln!("Error in argument {ind}, char {}: {msg}", chr + 1);
        }
        Opt::Invalid
    }

    pub(crate) fn next(&mut self, show: bool) -> Opt {
        let argc = self.args.len();
        if self.optind >= argc {
            return Opt::End;
        }
        let ind = self.optind;
        if !self.dash && (self.at(ind, 0) != b'-' || self.at(ind, 1) == 0) {
            return Opt::End;
        }
        let arg = self.args[ind].as_bytes();
        let mut idx: Option<usize> = None;
        let mut arg_start;
        if arg.len() >= 2 && arg[0] == b'-' && arg[1] == b'-' {
            if arg.len() == 2 {
                self.optind += 1;
                return Opt::End;
            }
            arg_start = 2;
            let mut arg_end = arg.len() - 1;
            match arg[arg_start..arg_end].iter().position(|&b| b == b'=') {
                Some(p) => {
                    arg_end = p;
                    arg_start += 1;
                }
                None => arg_end -= 1,
            }
            let name = &arg[2..(2 + arg_end).min(arg.len())];
            match OPTIONS.iter().position(|o| o.2.as_bytes() == name) {
                Some(i) => idx = Some(i),
                None => {
                    self.optind += 1;
                    return self.error(self.optind - 1, self.optchr, "arg", show);
                }
            }
            self.optchr = 0;
            self.dash = false;
            arg_start += OPTIONS[idx.unwrap_or(0)].2.len();
        } else {
            if !self.dash {
                self.dash = true;
                self.optchr = 1;
            }
            if self.at(ind, self.optchr) == b':' {
                self.dash = false;
                self.optind += 1;
                return self.error(self.optind - 1, self.optchr, "colon", show);
            }
            arg_start = 1 + self.optchr;
        }
        let i = match idx {
            Some(i) => i,
            None => {
                let c = self.at(ind, self.optchr);
                match OPTIONS.iter().position(|o| o.0 == c) {
                    Some(i) => i,
                    None => {
                        let (errind, errchr) = (ind, self.optchr);
                        if self.at(ind, self.optchr + 1) == 0 {
                            self.dash = false;
                            self.optind += 1;
                        } else {
                            self.optchr += 1;
                        }
                        return self.error(errind, errchr, "nf", show);
                    }
                }
            }
        };
        let (c, need_param, _) = OPTIONS[i];
        if need_param {
            self.dash = false;
            if self.at(ind, arg_start) == 0 {
                self.optind += 1;
                if self.optind == argc {
                    return self.error(self.optind - 1, self.optchr, "arg", show);
                }
                let v = self.args[self.optind].clone();
                self.optind += 1;
                return Opt::Char(c, Some(v));
            }
            if self.at(ind, arg_start) == b'=' {
                arg_start += 1;
            }
            let v = String::from_utf8_lossy(&arg[arg_start.min(arg.len())..]).into_owned();
            self.optind += 1;
            return Opt::Char(c, Some(v));
        }
        let long = arg.len() >= 2 && arg[0] == b'-' && arg[1] == b'-';
        if arg_start >= 2 && !long {
            if self.at(ind, self.optchr + 1) == 0 {
                self.dash = false;
                self.optind += 1;
            } else {
                self.optchr += 1;
            }
        } else {
            self.optind += 1;
        }
        Opt::Char(c, None)
    }
}
