//! A port of php's `php_getopt` (main/getopt.c), which php-cli and php-cgi
//! both parse their command lines with: bundled short options (`-qC`),
//! `-dfoo` / `-d foo` / `-d=foo` values, `--long[=value]` names, and its
//! error messages.

/// One option of a table (php's `opt_struct`).
pub(crate) struct OptDef {
    /// What [`PhpGetopt::next`] returns for it: its letter, or a number for
    /// a long-only option.
    pub(crate) ch: u8,
    /// [`NO_ARG`], [`REQUIRED`] or [`OPTIONAL`].
    pub(crate) param: u8,
    /// Its long name.
    pub(crate) name: &'static str,
}

/// The option takes no value.
pub(crate) const NO_ARG: u8 = 0;
/// The option needs a value.
pub(crate) const REQUIRED: u8 = 1;
/// The value is optional, and only given as `--name=value` or `-xvalue`.
pub(crate) const OPTIONAL: u8 = 2;

pub(crate) const fn opt(ch: u8, param: u8, name: &'static str) -> OptDef {
    OptDef { ch, param, name }
}

/// One `php_getopt` result.
pub(crate) enum Opt {
    /// No more options ([`PhpGetopt::optind`] is the first operand).
    End,
    /// An invalid option (reported on stderr when asked to).
    Invalid,
    /// An option and its value.
    Char(u8, Option<String>),
}

/// The errors `php_opt_error` reports.
enum OptError {
    /// `-:`.
    Colon,
    /// An unknown short option.
    NotFound,
    /// A missing value, or an unknown long option.
    Arg,
}

/// `php_getopt`'s state over one pass of the command line.
pub(crate) struct PhpGetopt<'a> {
    args: &'a [String],
    opts: &'a [OptDef],
    /// The argument being read.
    pub(crate) optind: usize,
    optchr: usize,
    dash: bool,
}

impl<'a> PhpGetopt<'a> {
    pub(crate) fn new(args: &'a [String], opts: &'a [OptDef]) -> Self {
        PhpGetopt {
            args,
            opts,
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

    fn error(&self, ind: usize, chr: usize, err: OptError, show: bool) -> Opt {
        if show {
            let c = self.at(ind, chr) as char;
            let msg = match err {
                OptError::Colon => ": in flags".to_string(),
                OptError::NotFound => format!("option not found {c}"),
                OptError::Arg => format!("no argument for option {c}"),
            };
            eprintln!("Error in argument {ind}, char {}: {msg}", chr + 1);
        }
        Opt::Invalid
    }

    /// The next option; `show` reports an invalid one on stderr.
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
            match self.opts.iter().position(|o| o.name.as_bytes() == name) {
                Some(i) => idx = Some(i),
                None => {
                    self.optind += 1;
                    return self.error(self.optind - 1, self.optchr, OptError::Arg, show);
                }
            }
            self.optchr = 0;
            self.dash = false;
            arg_start += self.opts[idx.unwrap_or(0)].name.len();
        } else {
            if !self.dash {
                self.dash = true;
                self.optchr = 1;
            }
            if self.at(ind, self.optchr) == b':' {
                self.dash = false;
                self.optind += 1;
                return self.error(self.optind - 1, self.optchr, OptError::Colon, show);
            }
            arg_start = 1 + self.optchr;
        }
        let i = match idx {
            Some(i) => i,
            None => {
                let c = self.at(ind, self.optchr);
                match self.opts.iter().position(|o| o.ch == c) {
                    Some(i) => i,
                    None => {
                        let (errind, errchr) = (ind, self.optchr);
                        if self.at(ind, self.optchr + 1) == 0 {
                            self.dash = false;
                            self.optind += 1;
                        } else {
                            self.optchr += 1;
                        }
                        return self.error(errind, errchr, OptError::NotFound, show);
                    }
                }
            }
        };
        let OptDef { ch: c, param, .. } = self.opts[i];
        if param != NO_ARG {
            self.dash = false;
            if self.at(ind, arg_start) == 0 {
                self.optind += 1;
                if self.optind == argc {
                    if param == REQUIRED {
                        return self.error(self.optind - 1, self.optchr, OptError::Arg, show);
                    }
                    return Opt::Char(c, None);
                }
                // An optional value is never the next argument.
                if param != REQUIRED {
                    return Opt::Char(c, None);
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

#[cfg(test)]
mod tests;
