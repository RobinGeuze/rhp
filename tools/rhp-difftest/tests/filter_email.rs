//! FILTER_VALIDATE_EMAIL against php. The check runs php's own regexes
//! through the PCRE2 php links, so which letters FILTER_FLAG_EMAIL_UNICODE
//! accepts follows that library's Unicode tables, and where the match limits
//! cut in follows its matcher and JIT. That differs between PCRE2 releases,
//! so these cases are compared live and are not goldens. Skips without `php`.

use rhp_difftest::{diff_source, php_available};

const CASES: &str = r#"
// Letters and digits from Unicode 15 to 17, unassigned, private use, emoji.
$chars = ["\u{16FF2}", "\u{16FF3}", "\u{10940}", "\u{11DB0}", "\u{1E6C0}", "\u{10D50}",
    "\u{10D70}", "\u{105C0}", "\u{11BC0}", "\u{16100}", "\u{10D40}", "\u{11BF0}", "\u{2EBF0}",
    "\u{11F04}", "\u{1E4D0}", "\u{31350}", "\u{0378}", "\u{E000}", "\u{1F600}"];
$in = [];
foreach ($chars as $c) {
    array_push($in, "$c@b.de", "a$c.b@c.de", "\"$c\"@b.de", "x@$c.de");
}
// Invalid UTF-8: stray bytes, truncated, surrogate, past U+10FFFF, overlong.
array_push($in, "\xff@b.com", "\xc3@b.com", "\xc3\x28@b.com", "\xe2\x82@b.com",
    "\xed\xa0\x80@b.com", "\xf4\x90\x80\x80@b.com", "\xc0\xaf@b.com", "a@b.com\xff",
    "\"\xff\"@b.com", "a\x80@b.com", "\xef\xbf\xbe@b.com");
// Under the u flag the regex counts characters, so 4-byte letters leave the
// 320-octet check to decide.
foreach ([319, 320, 321] as $n) {
    $in[] = str_repeat("\u{20000}", 64) . '@' . str_repeat('b', $n - 261) . '.com';
}
array_push($in, str_repeat("\u{e9}", 64) . '@b.com', str_repeat("\u{e9}", 65) . '@b.com');
// Quoted escapes backtrack until pcre.backtrack_limit stops the match.
for ($n = 6; $n <= 10; $n++) {
    $in[] = implode('.', array_fill(0, $n, '"\a"')) . '@b.com';
}
foreach ($in as $s) {
    echo strlen($s), ' ', bin2hex(substr($s, 0, 24)), ' ',
        var_export(filter_var($s, FILTER_VALIDATE_EMAIL) !== false, true), ' ',
        var_export(filter_var($s, FILTER_VALIDATE_EMAIL, FILTER_FLAG_EMAIL_UNICODE) !== false, true), "\n";
}
// filter_var neither resets nor sets preg_last_error().
preg_match('/a/u', "\xff");
var_dump(filter_var('a@b.com', FILTER_VALIDATE_EMAIL), preg_last_error());
var_dump(filter_var("\xff@b.com", FILTER_VALIDATE_EMAIL, FILTER_FLAG_EMAIL_UNICODE), preg_last_error());
preg_match('/a/', 'a');
var_dump(filter_var(implode('.', array_fill(0, 10, '"\a"')) . '@b.com', FILTER_VALIDATE_EMAIL), preg_last_error());
// The filter's match goes through the pcre.* match limits.
ini_set('pcre.backtrack_limit', '30');
var_dump(filter_var('a@b.com', FILTER_VALIDATE_EMAIL), preg_last_error());
"#;

fn check(name: &str, prelude: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    let src = format!("<?php\n{prelude}\n{CASES}");
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

#[test]
fn filter_email_matches_php() {
    check("filter_email", "");
}

/// Set before the first filter call, so the cached regexes compile without
/// the JIT.
#[test]
fn filter_email_without_jit_matches_php() {
    check("filter_email_nojit", "ini_set('pcre.jit', '0');");
}
