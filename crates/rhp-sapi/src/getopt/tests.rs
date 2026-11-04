use super::*;

const OPTS: &[OptDef] = &[
    opt(b'd', REQUIRED, "define"),
    opt(b'q', NO_ARG, "quiet"),
    opt(15, OPTIONAL, "ini"),
];

/// Every option of one pass, then the first operand's index.
fn parse(args: &[&str]) -> (Vec<(u8, Option<String>)>, usize) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut g = PhpGetopt::new(&args, OPTS);
    let mut got = Vec::new();
    loop {
        match g.next(false) {
            Opt::End => return (got, g.optind),
            Opt::Invalid => got.push((b'!', None)),
            Opt::Char(c, v) => got.push((c, v)),
        }
    }
}

#[test]
fn values_attached_separate_and_after_equals() {
    let (got, optind) = parse(&["php", "-dx=1", "-d", "y", "-d=z", "--define=w=2", "s.php"]);
    let vals: Vec<_> = got.iter().map(|(_, v)| v.as_deref().unwrap()).collect();
    assert_eq!(vals, ["x=1", "y", "z", "w=2"]);
    assert_eq!(optind, 6);
}

#[test]
fn an_optional_value_is_never_the_next_argument() {
    assert_eq!(parse(&["php", "--ini"]).0, [(15, None)]);
    assert_eq!(parse(&["php", "--ini=diff"]).0, [(15, Some("diff".into()))]);
    // `--ini diff`: no value, and `diff` is the first operand.
    assert_eq!(parse(&["php", "--ini", "diff"]), (vec![(15, None)], 2));
}

#[test]
fn clusters_and_the_end_of_options() {
    let (got, optind) = parse(&["php", "-qq", "--", "-q"]);
    assert_eq!(got, [(b'q', None), (b'q', None)]);
    assert_eq!(optind, 3);
    // A lone `-` is an operand (standard input).
    assert_eq!(parse(&["php", "-"]), (vec![], 1));
    // An unknown option in a cluster, then the rest of the cluster.
    assert_eq!(parse(&["php", "-xq"]).0, [(b'!', None), (b'q', None)]);
}
