//! ext/iconv against php. rhp calls the iconv(3) php links (the macOS
//! SDK's libiconv, glibc's on Linux), so conversions, error positions and
//! //TRANSLIT / //IGNORE results match the php on the same machine, but they
//! differ between the two libraries (UTF-16 and UCS-2 byte order,
//! transliterations, which characters a charset has), so these cases cannot
//! be goldens. macOS's libiconv gives a multi-character //TRANSLIT string
//! different results from call to call, so there is none here. Skips without
//! `php`.

use rhp_difftest::{diff_source, php_available};

fn check(name: &str, src: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

/// Prints warnings and notices: the harness runs php with error_reporting=0
/// (and rhp without it, so nothing may be raised before this).
const PRELUDE: &str = r#"<?php
set_error_handler(function ($no, $str) { echo "  W[$no]: $str\n"; return true; });
"#;

#[test]
fn conversions_match_php() {
    check(
        "iconv",
        &(PRELUDE.to_string()
            + r##"
$cases = [
    ["UTF-8", "ASCII//TRANSLIT", "Ω"], ["UTF-8", "ASCII//TRANSLIT", "中"], ["UTF-8", "ASCII//TRANSLIT", "é"],
    ["UTF-8", "ASCII//TRANSLIT", "€"], ["UTF-8", "ASCII//IGNORE", "aé b"], ["UTF-8", "ASCII//TRANSLIT//IGNORE", "a中b"],
    ["UTF-8", "ASCII", "aé"], ["UTF-8", "ISO-8859-1//TRANSLIT", "Ω€"], ["UTF-8", "ISO-8859-1//TRANSLIT", "€"],
    ["UTF-8", "ISO-8859-1//IGNORE", "aΩb€c"], ["UTF-8", "ISO-8859-1//IGNORE//TRANSLIT", "aΩb"],
    ["UTF-8", "SJIS", "日本語"], ["UTF-8", "CP932", "～①"], ["UTF-8", "EUC-JP", "～"], ["UTF-8", "GB18030", "\u{10000}"],
    ["UTF-8", "BIG5", "€"], ["UTF-8", "UTF-16", "A"], ["UTF-8", "UTF-32", "A"], ["UTF-8", "UCS-2", "A"],
    ["UTF-8", "UCS-4", "A"], ["UTF-16", "UTF-8", "\xfe\xff\x00A"], ["UTF-16", "UTF-8", "\xff\xfeA\x00"],
    ["SJIS", "UTF-8", "\x81\x60"], ["CP932", "UTF-8", "\x81\x60"], ["UTF-8", "UTF-8", "a\xffb"],
    ["UTF-8", "UTF-8//IGNORE", "a\xffb"], ["UTF-8", "UTF-8//IGNORE", "\xff"], ["UTF-8", "UTF-8//IGNORE", "a\xff"],
    ["UTF-8", "ASCII", "a\xc3"], ["UTF-8", "MACINTOSH", "é"], ["UTF-8", "KOI8-R", "я"], ["UTF-8", "bogus", "x"],
    ["bogus", "UTF-8", "x"], ["UTF-8", "ISO-2022-JP", "a日本b"], ["ISO-2022-JP", "UTF-8", "\x1b\$B\x46\x7c\x1b(B"],
    ["ISO-2022-JP", "UTF-8", "\x1b\$B\x46"], ["UTF-16LE", "UTF-8", "a\x00\x00\xd8"], ["UTF-16LE", "UTF-8", "a\x00\x00"],
    ["UTF-8", "UTF-7", "Hi é"], ["UTF-7", "UTF-8", "Hi +AOk-"], ["UTF-8\0junk", "ASCII", "x"], ["UTF-8", "ASCII\0x", "y"],
    [str_repeat("x", 64), "ASCII", "x"], ["UTF-8", str_repeat("x", 63), "x"], ["", "UTF-8", "x"], ["UTF-8", "", "x"],
    ["latin1", "utf-8", "\xe9t\xe9"], ["UTF-8", "ISO-8859-15", "€"], ["UTF-8", "CP1252", "€‚"], ["CP1252", "UTF-8", "\x81"],
    ["UTF-8", "ASCII", ""], ["UTF-8", "UTF-16", ""], ["UTF-8", "ASCII", "a\0b"], ["UTF-8", "UTF-8", "\xed\xa0\x80"],
    ["UTF-8", "UTF-8", "\xf4\x90\x80\x80"], ["UTF-8", "UTF-8", "\xc0\x80"], ["UTF-8", "ASCII", str_repeat("é", 40)],
    ["UTF-8", "UTF-32LE", str_repeat("ab", 300)], ["ASCII", "UTF-16BE", str_repeat("z", 1000)],
];
foreach ($cases as [$f, $t, $s]) {
    $r = iconv($f, $t, $s);
    printf("%s -> %s %s => %s\n", json_encode($f), json_encode($t), strlen($s) > 64 ? md5($s) : bin2hex($s), $r === false ? 'false' : (strlen($r) > 64 ? md5($r) . '/' . strlen($r) : bin2hex($r)));
}
"##),
    );
}

#[test]
fn counting_functions_match_php() {
    check(
        "iconv_strlen/substr/strpos",
        &(PRELUDE.to_string()
            + r##"
function strpos_or_error(...$a) { try { return iconv_strpos(...$a); } catch (ValueError $e) { return $e->getMessage(); } }
$text = "aé日本語bé€x";
foreach (["UTF-8", "SJIS", "EUC-JP", "UTF-16", "UTF-16LE", "UTF-16BE", "ISO-2022-JP", "UCS-2BE", "UCS-4LE",
          "GB18030", "ISO-8859-1", "ASCII", "CP1252", "UTF-7", "BIG5", "bogus"] as $enc) {
    $s = @iconv("UTF-8", $enc . "//IGNORE", $text);
    echo "== $enc ", $s === false ? 'false' : bin2hex($s), "\n";
    if ($s === false) { $s = "abc"; }
    $b = @iconv("UTF-8", $enc, "b");
    $e = @iconv("UTF-8", $enc, "é");
    var_dump(iconv_strlen($s, $enc), iconv_substr($s, 1, 3, $enc) === false ? false : bin2hex(iconv_substr($s, 1, 3, $enc)));
    var_dump(bin2hex((string) iconv_substr($s, -2, null, $enc)), iconv_substr($s, 99, 1, $enc), bin2hex((string) iconv_substr($s, 0, -1, $enc)));
    if ($b !== false) { var_dump(iconv_strpos($s, $b, 0, $enc), strpos_or_error($s, $b, 4, $enc), iconv_strrpos($s, $b, $enc)); }
    if ($e !== false) { var_dump(iconv_strpos($s, $e, 0, $enc), iconv_strrpos($s, $e, $enc), strpos_or_error($s, $e, -3, $enc)); }
    foreach (["\xff", "\xff\xfe\xfd", "\x80", "\xe3", "\x1b\$B"] as $junk) {
        $bad = $s . $junk;
        echo "  junk ", bin2hex($junk), "\n";
        var_dump(iconv_strlen($bad, $enc), iconv_substr($bad, 0, 2, $enc) === false ? false : bin2hex(iconv_substr($bad, 0, 2, $enc)));
        if ($b !== false) { var_dump(iconv_strpos($bad, $b, 0, $enc), iconv_strrpos($bad, $b, $enc)); }
        $mid = substr($s, 0, 1) . $junk . substr($s, 1);
        var_dump(iconv_strlen($mid, $enc));
        if ($b !== false) { var_dump(iconv_strpos($mid, $b, 0, $enc), iconv_strrpos($mid, $b, $enc)); }
    }
}
try { var_dump(iconv_strpos("abc", "a", -4)); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { var_dump(iconv_strpos("abc", "a", 4)); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(iconv_strpos("abc", "", 0), iconv_strrpos("abc", ""), iconv_strpos("abcabc", "ca"), iconv_strrpos("abcabc", "ca"));
var_dump(iconv_strpos("aaab", "aab"), iconv_strpos("ababac", "abac"), iconv_strrpos("aaaa", "aa"));
var_dump(iconv_substr("abc", 1, 0), iconv_substr("", 0), iconv_substr("abc", -5, 2), iconv_substr("abc", 1, -5));
var_dump(iconv_strlen("abc", str_repeat("x", 64)), iconv_substr("abc", 0, 1, str_repeat("x", 64)));
var_dump(iconv_strlen("abc", "UTF-8\0junk"), iconv_strlen("abc", ""));
"##),
    );
}

#[test]
fn mime_functions_match_php() {
    check(
        "iconv_mime",
        &(PRELUDE.to_string()
            + r##"
$values = ["Prüfung", "Hello", str_repeat("é", 50), str_repeat("日本語のテキスト", 3), "a b  c", "x=?y?=z_", "",
           "ä\x00b", str_repeat("abcdefghij", 20), "\xff"];
foreach ($values as $v) {
    foreach (["B", "Q"] as $scheme) {
        foreach (["UTF-8", "ISO-8859-1", "ISO-2022-JP", "UTF-16", "UTF-7"] as $out) {
            foreach ([76, 20, 1000] as $ll) {
                $r = iconv_mime_encode("Subject", $v, ["scheme" => $scheme, "output-charset" => $out, "line-length" => $ll]);
                echo json_encode([$scheme, $out, $ll, bin2hex($v)]), " => ", json_encode($r), "\n";
            }
        }
    }
}
$opts = [
    [], ["scheme" => "q"], ["scheme" => "x"], ["scheme" => ""], ["scheme" => 5], ["line-break-chars" => "\n"],
    ["line-break-chars" => " "], ["line-break-chars" => 7], ["line-break-chars" => ["a"]], ["line-length" => "40"],
    ["line-length" => 0], ["line-length" => 15], ["line-length" => 22], ["input-charset" => str_repeat("x", 64)],
    ["output-charset" => str_repeat("x", 64)], ["input-charset" => "ISO-8859-1"], ["input-charset" => "bogus"],
    ["output-charset" => "bogus"], ["input-charset" => ""], ["line-break-chars" => "a\0b"],
];
foreach ($opts as $o) {
    $r = iconv_mime_encode("Subject", "Prüfung über alles", $o);
    echo json_encode($o), " => ", json_encode($r), "\n";
}
var_dump(iconv_mime_encode(str_repeat("N", 80), "x"), iconv_mime_encode("Fé", "x"), iconv_mime_encode("", ""));
$enc = [
    "Subject: =?UTF-8?Q?Pr=C3=BCfung?=", "Subject: =?UTF-8?B?UHLDvGZ1bmc=?=", "=?ISO-8859-1?Q?a?= =?ISO-8859-1?Q?b?=",
    "=?ISO-8859-1?Q?a?=  x  =?ISO-8859-1?Q?b?=", "=?UTF-8?Q?a?==?UTF-8?Q?b?=", "=?UTF-8?Q?a?=x", "=?UTF-8*en?Q?a_b?=",
    "a\r\n b\r\n\tc", "a\r\nb: c", "a\rb", "=?UTF-8?X?abc?=", "=?FOO?Q?abc?= x", "=?UTF-8?Q?\xff?=", "=?UTF-8?Q?=ZZ?=",
    "=?UTF-8?B?***?=", "=?UTF-8?Q?abc", "=?UTF-8?Q", "=?", "=", "=x", "=?UTF-8?Q?abc?\r\n", "=?" . str_repeat("C", 90) . "?Q?a?=",
    "=?UTF-8\r\n?Q?a?=", "=?\nx", "Subject: =?ISO-2022-JP?B?GyRCRnxLXDhsGyhC?=", "=?UTF-16?B?/v8AQQ==?=", "x \xe9 y",
    "=?UTF-8?Q?=E3=81?=", "=?utf-8?q?=e3=81=82?=", "=?UTF-8?Q?a=\r\n b?=", "=?UTF-8?Q?a?= \r\n =?UTF-8?Q?b?=",
];
foreach ($enc as $e) {
    foreach ([0, 1, 2, 3] as $mode) {
        foreach (["UTF-8", "ISO-8859-1"] as $cs) {
            $r = iconv_mime_decode($e, $mode, $cs);
            echo json_encode([bin2hex($e), $mode, $cs]), " => ", $r === false ? "false" : bin2hex($r), "\n";
        }
    }
}
var_dump(iconv_mime_decode("x", 0, "bogus"), iconv_mime_decode("x", 0, str_repeat("x", 64)));
$headers = [
    "A: 1\r\nB: 2\r\nA: 3\r\nA: 4\r\n", "Subject: =?UTF-8?Q?Pr=C3=BCfung?=\r\nTo: x\r\n\r\nbody", "A: 1\r\n folded\r\n\tmore\r\nB:x",
    "noheader\r\nA: b\r\n", "\r\n\r\nA: b", "123: a\r\n123: b\r\n", "A:  \t v \r\n", "Bad: =?UTF-8?X?a?=\r\nC: d",
    "a:b\nc:d\n", ":empty\r\n",
];
foreach ($headers as $h) {
    foreach ([0, 2] as $mode) {
        echo json_encode([bin2hex($h), $mode]), " => ", json_encode(iconv_mime_decode_headers($h, $mode, "UTF-8")), "\n";
    }
}
"##),
    );
}

/// Z_PARAM_LONG's deprecations and TypeErrors, and zval_get_long on the
/// `line-length` option.
#[test]
fn int_arguments_match_php() {
    let cases = r##"
function show(callable $f) {
    try { var_dump($f()); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
show(fn() => iconv_substr('abcdef', 1.5, 2.5));
show(fn() => iconv_substr('abcdef', '1.5'));
show(fn() => iconv_substr('abcdef', 1, null));
show(fn() => iconv_substr('abcdef', 1, 2.0));
show(fn() => iconv_substr('abcdef', '1e30'));
show(fn() => iconv_substr('abcdef', 1e30));
show(fn() => iconv_substr('abcdef', 1, 'x'));
show(fn() => iconv_substr('abcdef', true));
show(fn() => iconv_strpos('abcdef', 'c', 1.5));
show(fn() => iconv_strpos('abcdef', 'c', '1'));
show(fn() => iconv_mime_decode('a', 1.5));
show(fn() => iconv_mime_decode_headers("a: b", 1.5));
"##;
    check(
        "int arguments",
        &(PRELUDE.to_string()
            + cases
            + r##"
show(fn() => iconv_substr('abcdef', null));
show(fn() => iconv_strpos('abcdef', 'c', null));
foreach ([new stdClass, function () {}, INF, NAN, 18446744073709551616.0 + 40, '1e999', '20abc', ' 30', 'x', [], [1],
          true, 2.7, '2.7'] as $ll) {
    show(fn() => iconv_mime_encode('Subject', 'Prüfung Prüfung',
        ['line-length' => $ll, 'scheme' => 'Q', 'input-charset' => 'UTF-8', 'output-charset' => 'UTF-8']));
}
"##),
    );
    check(
        "int arguments, strict_types",
        &("<?php declare(strict_types=1);\n".to_string() + &PRELUDE[5..] + cases),
    );
}

#[test]
fn encodings_and_ini_match_php() {
    check(
        "iconv_get/set_encoding",
        &(PRELUDE.to_string()
            + r##"
var_dump(iconv_get_encoding(), iconv_get_encoding('internal_encoding'), iconv_get_encoding('ALL'), iconv_get_encoding('bogus'));
var_dump(ini_set('default_charset', 'ISO-8859-1'), iconv_get_encoding(), iconv_strlen("\xe9t\xe9"), iconv_mime_decode("=?UTF-8?Q?=C3=A9?="));
var_dump(ini_set('internal_encoding', 'UTF-8'), iconv_get_encoding('all'), iconv_strlen("\xe9t\xe9"));
var_dump(iconv_set_encoding('internal_encoding', 'EUC-JP'), iconv_set_encoding('bogus', 'x'), iconv_set_encoding('output_encoding', str_repeat('x', 64)));
var_dump(ini_get('iconv.internal_encoding'), iconv_get_encoding());
var_dump(ini_set('iconv.input_encoding', str_repeat('y', 64)), ini_get('iconv.input_encoding'), ini_set('iconv.input_encoding', 'SJIS'));
var_dump(ini_alter('iconv.output_encoding', 'UTF-16'), iconv_set_encoding('INPUT_ENCODING', 'UTF-8'));
var_dump(ini_set('default_charset', "a\nb"), ini_set('default_charset', "a\0b"), ini_get('default_charset'));
ini_restore('iconv.internal_encoding');
var_dump(iconv_get_encoding());
var_dump(iconv_mime_encode("S", "\xa4\xa2"));
ini_set('internal_encoding', '');
ini_set('iconv.internal_encoding', '');
ini_set('default_charset', '');
var_dump(iconv_get_encoding(), iconv_strlen("é"));
var_dump(ICONV_IMPL === (PHP_OS_FAMILY === 'Darwin' ? 'libiconv' : 'glibc'), is_string(ICONV_VERSION), ICONV_MIME_DECODE_STRICT, ICONV_MIME_DECODE_CONTINUE_ON_ERROR);
var_dump(function_exists("ob_iconv_handler"), function_exists("iconv_mime_encode"));
@iconv('UTF-8', 'ASCII', "\xc3");
var_dump(iconv_mime_decode("=?UTF-8?Q?=ZZ?="));
@iconv('UTF-8', 'ASCII', "\xc3\xa9");
var_dump(iconv_mime_decode("=?UTF-8?Q?=ZZ?="));
"##),
    );
}

/// ob_iconv_handler. It refuses to start once output went out, so the
/// script sends nothing before its first buffer.
#[test]
fn output_handler_matches_php() {
    check(
        "ob_iconv_handler",
        r##"<?php
@iconv_set_encoding('internal_encoding', 'UTF-8');
@iconv_set_encoding('output_encoding', 'ISO-8859-1');
ob_start(function ($b) { return bin2hex($b) . "\n"; });
set_error_handler(function ($no, $str) { echo "  W[$no]: $str\n"; return true; });
ob_start('ob_iconv_handler');
echo "é";
$s = ob_get_status(true);
var_dump(ob_start('ob_iconv_handler'));
ob_end_flush();
echo "|";
var_dump(count($s), $s[1]['name'], $s[1]['type'], $s[1]['flags']);
ob_start('ob_iconv_handler');
echo "ü€";
ob_flush();
var_dump(ob_get_status()['flags'], ob_end_clean());
echo "ä";
var_dump(ob_get_level(), ob_list_handlers()[1]);
iconv_set_encoding('output_encoding', 'ASCII//TRANSLIT');
echo "Ω";
ob_end_flush();
ob_end_flush();
echo "x";
ob_start('ob_iconv_handler');
echo "é";
ob_end_flush();
echo "\n";
ob_start('ob_iconv_handler', 4);
echo "abcdéfgh";
ob_end_clean();
var_dump(ob_start('OB_ICONV_HANDLER'));
"##,
    );
}
