//! The php-cli front end (sapi/cli/php_cli.c): its option table, usage
//! text and the passes `main` and `do_cli` make over the command line, so
//! `rhp` reads options, and reports bad ones, as php does. The modes rhp
//! does not have (`-i`, `-l`, `-m`, `-s`, `-w`, `-B`/`-R`/`-F`/`-E`,
//! `--rf` and friends, `--ini`, `--repeat`) are refused once php would run
//! them, after its own checks. `-a` is an error, as in a php without
//! readline.

use crate::getopt::{opt, Opt, OptDef, PhpGetopt, NO_ARG, OPTIONAL, REQUIRED};
use crate::Script;

/// php-cli's `OPTIONS`.
pub(crate) const OPTIONS: &[OptDef] = &[
    opt(b'a', NO_ARG, "interactive"),
    opt(b'B', REQUIRED, "process-begin"),
    opt(b'C', NO_ARG, "no-chdir"),
    opt(b'c', REQUIRED, "php-ini"),
    opt(b'd', REQUIRED, "define"),
    opt(b'E', REQUIRED, "process-end"),
    opt(b'e', NO_ARG, "profile-info"),
    opt(b'F', REQUIRED, "process-file"),
    opt(b'f', REQUIRED, "file"),
    opt(b'h', NO_ARG, "help"),
    opt(b'i', NO_ARG, "info"),
    opt(b'l', NO_ARG, "syntax-check"),
    opt(b'm', NO_ARG, "modules"),
    opt(b'n', NO_ARG, "no-php-ini"),
    opt(b'q', NO_ARG, "no-header"),
    opt(b'R', REQUIRED, "process-code"),
    opt(b'H', NO_ARG, "hide-args"),
    opt(b'r', REQUIRED, "run"),
    opt(b's', NO_ARG, "syntax-highlight"),
    opt(b's', NO_ARG, "syntax-highlighting"),
    opt(b'S', REQUIRED, "server"),
    opt(b't', REQUIRED, "docroot"),
    opt(b'w', NO_ARG, "strip"),
    opt(b'?', NO_ARG, "usage"),
    opt(b'v', NO_ARG, "version"),
    opt(10, REQUIRED, "rf"),
    opt(10, REQUIRED, "rfunction"),
    opt(11, REQUIRED, "rc"),
    opt(11, REQUIRED, "rclass"),
    opt(12, REQUIRED, "re"),
    opt(12, REQUIRED, "rextension"),
    opt(13, REQUIRED, "rz"),
    opt(13, REQUIRED, "rzendextension"),
    opt(14, REQUIRED, "ri"),
    opt(14, REQUIRED, "rextinfo"),
    opt(15, OPTIONAL, "ini"),
    opt(16, REQUIRED, "repeat"),
];

/// php_cli_usage: `prog` is the program name after its last `/`, else
/// "php".
fn usage(argv0: &str) -> String {
    let prog = argv0.rsplit_once('/').map_or("php", |(_, p)| p);
    format!(
        "Usage: {prog} [options] [-f] <file> [--] [args...]\n   {prog} [options] -r <code> [--] [args...]\n   {prog} [options] [-B <begin_code>] -R <code> [-E <end_code>] [--] [args...]\n   {prog} [options] [-B <begin_code>] -F <file> [-E <end_code>] [--] [args...]\n   {prog} [options] -S <addr>:<port> [-t docroot] [router]\n   {prog} [options] -- [args...]\n   {prog} [options] -a\n\n  -a               Run as interactive shell (requires readline extension)\n  -c <path>|<file> Look for php.ini file in this directory\n  -n               No configuration (ini) files will be used\n  -d foo[=bar]     Define INI entry foo with value 'bar'\n  -e               Generate extended information for debugger/profiler\n  -f <file>        Parse and execute <file>.\n  -h               This help\n  -i               PHP information\n  -l               Syntax check only (lint)\n  -m               Show compiled in modules\n  -r <code>        Run PHP <code> without using script tags <?..?>\n  -B <begin_code>  Run PHP <begin_code> before processing input lines\n  -R <code>        Run PHP <code> for every input line\n  -F <file>        Parse and execute <file> for every input line\n  -E <end_code>    Run PHP <end_code> after processing all input lines\n  -H               Hide any passed arguments from external tools.\n  -S <addr>:<port> Run with built-in web server.\n  -t <docroot>     Specify document root <docroot> for built-in web server.\n  -s               Output HTML syntax highlighted source.\n  -v               Version number\n  -w               Output source with stripped comments and whitespace.\n\n  args...          Arguments passed to script. Use -- args when first argument\n                   starts with - or script is read from stdin\n\n  --ini            Show configuration file names\n  --ini=diff       Show INI entries that differ from the built-in default\n\n  --rf <name>      Show information about function <name>.\n  --rc <name>      Show information about class <name>.\n  --re <name>      Show information about extension <name>.\n  --rz <name>      Show information about Zend extension <name>.\n  --ri <name>      Show configuration for extension <name>.\n\n  --repeat <count> Repeat script execution <count> times.\n                   For internal purposes only.\n\n"
    )
}

const MODE_CONFLICT: &str = "Either execute direct code, process stdin or use a file.\n";

/// What `do_cli` was asked to do (php's `PHP_CLI_MODE_*`, the reflection
/// and ini modes folded).
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Standard,
    Highlight,
    Lint,
    Strip,
    Direct,
    ProcessStdin,
    Reflection,
    ShowIni,
}

/// Refuse a mode or option rhp does not have: `c` is its letter, or a
/// long-only option's number.
fn refuse(c: u8) -> i32 {
    let name = match c {
        b' '..=b'~' => format!("-{}", c as char),
        _ => {
            let long = OPTIONS.iter().find(|o| o.ch == c).map_or("", |o| o.name);
            format!("--{long}")
        }
    };
    eprintln!("rhp: option {name} is not supported");
    1
}

/// php-cli: parse `args` (`args[0]` is the program name) as php's `main`
/// and `do_cli` do, then run the script. Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    let raw: Vec<Vec<u8>> = args.iter().map(|a| a.as_bytes().to_vec()).collect();
    main_bytes(&raw)
}

/// [`main`] for the process's own arguments, which need not be UTF-8: the
/// script's `$argv` keeps their bytes; options are parsed from a lossy
/// UTF-8 copy.
pub fn main_bytes(raw: &[Vec<u8>]) -> i32 {
    let args: Vec<String> = raw
        .iter()
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let args = &args[..];
    let argv0 = args.first().map_or("", String::as_str);

    // main(): the ini settings, help, and invalid options, which only this
    // pass reports. -i, -v and -m end it.
    let mut ini = Vec::new();
    let mut ini_files = crate::IniFiles::default();
    let mut server = false;
    let mut g = PhpGetopt::new(args, OPTIONS);
    loop {
        match g.next(true) {
            Opt::End => break,
            Opt::Invalid => {
                print!("{}", usage(argv0));
                return 1;
            }
            Opt::Char(b'h' | b'?', _) => {
                print!("{}", usage(argv0));
                return 0;
            }
            Opt::Char(b'd', Some(kv)) => ini.push(crate::ini_define(&kv)),
            Opt::Char(b'c', Some(path)) => ini_files.path_override = Some(path),
            Opt::Char(b'n', _) => ini_files.ignore = true,
            Opt::Char(b'S', _) => server = true,
            Opt::Char(b'i' | b'v' | b'm', _) => {
                server = false;
                break;
            }
            // -e: extended info for debuggers.
            Opt::Char(..) => {}
        }
    }
    if server {
        return crate::cli_server::main(args, ini, ini_files, crate::php_binary());
    }

    // do_cli()'s first pass: -i, -v and -m, whichever comes first.
    let mut g = PhpGetopt::new(args, OPTIONS);
    loop {
        match g.next(false) {
            Opt::End => break,
            // Both after module startup (its warnings come first), with no
            // script and no auto_prepend_file.
            Opt::Char(b'v', _) => {
                // A literal, so disable_functions cannot get in the way.
                let banner = crate::version_banner("cli");
                let lit = banner.replace('\\', "\\\\").replace('\'', "\\'");
                let code = format!("echo '{lit}';");
                return crate::run_cli(Script::Code(code), ini, Vec::new(), ini_files);
            }
            Opt::Char(b'i', _) => {
                let code = "phpinfo(INFO_ALL & ~INFO_CREDITS);".to_string();
                return crate::run_cli(Script::Code(code), ini, Vec::new(), ini_files);
            }
            Opt::Char(c @ b'm', _) => return refuse(c),
            _ => {}
        }
    }

    // The second pass: the mode, the script and the code, and the errors
    // in combining them. The last error wins.
    let mut mode = Mode::Standard;
    // The option that chose `mode` (for a refusal).
    let mut mode_opt = 0u8;
    let mut param_error: Option<&str> = None;
    let mut script_file: Option<String> = None;
    let mut exec_direct: Option<String> = None;
    let (mut exec_run, mut exec_begin, mut exec_end) = (false, false, false);
    let mut hide_argv = false;
    let mut num_repeats = 1;
    let mut g = PhpGetopt::new(args, OPTIONS);
    loop {
        let (c, arg) = match g.next(false) {
            Opt::End => break,
            Opt::Invalid => continue,
            Opt::Char(c, arg) => (c, arg),
        };
        match c {
            // rhp has no readline (extension_loaded('readline') is false),
            // so this is php without it, and php's `interactive` is never
            // set. The reference build has readline and CI's php does not,
            // so -a can have no CLI fixture.
            b'a' => param_error = Some("Interactive shell (-a) requires the readline extension.\n"),
            b'F' | b'R' => {
                if mode == Mode::ProcessStdin {
                    if exec_run || script_file.is_some() {
                        param_error = Some("You can use -R or -F only once.\n");
                        continue;
                    }
                } else if mode != Mode::Standard {
                    param_error = Some(MODE_CONFLICT);
                    continue;
                }
                (mode, mode_opt) = (Mode::ProcessStdin, c);
                if c == b'F' {
                    script_file = arg;
                } else {
                    exec_run = true;
                }
            }
            b'f' => {
                if matches!(mode, Mode::Direct | Mode::ProcessStdin) {
                    param_error = Some(MODE_CONFLICT);
                    continue;
                } else if script_file.is_some() {
                    param_error = Some("You can use -f only once.\n");
                    continue;
                }
                script_file = arg;
            }
            b'l' => {
                if mode == Mode::Standard {
                    (mode, mode_opt) = (Mode::Lint, c);
                }
            }
            b'r' => {
                if mode == Mode::Direct {
                    if exec_direct.is_some() || script_file.is_some() {
                        param_error = Some("You can use -r only once.\n");
                        continue;
                    }
                } else if mode != Mode::Standard {
                    param_error = Some(MODE_CONFLICT);
                    continue;
                }
                mode = Mode::Direct;
                exec_direct = arg;
            }
            b'B' | b'E' => {
                let seen = if c == b'B' {
                    &mut exec_begin
                } else {
                    &mut exec_end
                };
                if mode == Mode::ProcessStdin {
                    if *seen {
                        param_error = Some(if c == b'B' {
                            "You can use -B only once.\n"
                        } else {
                            "You can use -E only once.\n"
                        });
                        continue;
                    }
                } else if mode != Mode::Standard {
                    param_error = Some(MODE_CONFLICT);
                    continue;
                }
                (mode, mode_opt) = (Mode::ProcessStdin, c);
                *seen = true;
            }
            b's' | b'w' => {
                if matches!(mode, Mode::Direct | Mode::ProcessStdin) {
                    param_error = Some(if c == b's' {
                        "Source highlighting only works for files.\n"
                    } else {
                        "Source stripping only works for files.\n"
                    });
                    continue;
                }
                let m = if c == b's' {
                    Mode::Highlight
                } else {
                    Mode::Strip
                };
                (mode, mode_opt) = (m, c);
            }
            b'H' => hide_argv = true,
            10..=14 => (mode, mode_opt) = (Mode::Reflection, c),
            15 => match arg.as_deref() {
                None | Some("diff") => (mode, mode_opt) = (Mode::ShowIni, c),
                Some(_) => param_error = Some("Unknown argument for --ini\n"),
            },
            16 => num_repeats = rhp_builtins::c_atoi(arg.unwrap_or_default().as_bytes()),
            // -C and -q are no-ops in php-cli too.
            _ => {}
        }
    }
    if let Some(e) = param_error {
        print!("{e}");
        return 1;
    }
    if !matches!(mode, Mode::Standard | Mode::Direct) {
        return refuse(mode_opt);
    }
    if num_repeats > 1 {
        return refuse(16);
    }

    // The first operand is the script, unless `--` came before it.
    let mut optind = g.optind;
    if optind < args.len()
        && script_file.is_none()
        && mode != Mode::Direct
        && args[optind - 1] != "--"
    {
        script_file = Some(args[optind].clone());
        optind += 1;
    }
    let script = match (script_file, exec_direct) {
        (Some(file), None) => Script::File(file),
        // php wipes its arguments for -H before it runs, the -r code
        // among them.
        (None, Some(code)) => Script::Code(if hide_argv { String::new() } else { code }),
        (None, None) => Script::Stdin,
        // php runs the code with the file as `$argv[0]` and
        // `SCRIPT_FILENAME`.
        (Some(_), Some(_)) => {
            eprintln!("rhp: -r after -f is not supported");
            return 1;
        }
    };
    let rest = raw[optind..].to_vec();
    crate::run_cli(script, ini, rest, ini_files)
}
