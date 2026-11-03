//! ext/xml, XMLWriter and the libxml glue against php. rhp links the libxml2
//! php links, so error codes, messages, positions and recovery match the php
//! on the same machine, but they differ between libxml2 releases (the mac's
//! 2.9.13 and CI's 2.15.2 disagree on several of these documents), so these
//! cases cannot be goldens. Skips without `php`.

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

/// Prints warnings and notices: the harness runs php with error_reporting=0.
const PRELUDE: &str = r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$docs = [
    '<a><b></a>', '<a>&foo;</a>', '<?xml version="1.0" encoding="bogus"?><a/>', '<a x="1" x="2"/>',
    '<a>', "<a>\xFF\xFE</a>", '<p:a/>', "<!DOCTYPE a [<!ELEMENT a (b)>]>\n<a/>", '<a>&#0;</a>',
    '<a></a><b/>', '<a>x</a><b/>', '<?xml version="1.0" encoding="UTF-16"?><a/>', '<a><![CDATA[x</a>',
    '<a b=c/>', '<a><!--x--y--></a>', '<a xmlns:p=""/>', "<?xml version=\"1.0\"?>\n<root>\n<x a=\"1\">\n<y>",
    '<!DOCTYPE a [<!ENTITY e "<b>">]><a>&e;</a>', "<a>\x01</a>", '<?xml version="1.1"?><a/>',
    '<?xml version="2.0"?><a/>', "<a attr=\"&lt;x\" b=\"v\x00\"/>", "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xE9</a>",
    '<!DOCTYPE a [<!ENTITY e SYSTEM "nofile.ent">]><a>&e;</a>', '<a>&#xD800;</a>',
    '<?xml version="1.0" standalone="maybe"?><a/>', '', ' ', 'abc', '<', '<a', '<a b="1"', '<1a/>',
    '<a/><?xml version="1.0"?>', '<a>]]></a>', '<a b="<"/>', "<a>\n  <b c='1'>\n</a>",
];
"#;

#[test]
fn xml_parser_errors_match_php() {
    check(
        "xml_parser_errors",
        &(PRELUDE.to_string()
            + r#"
foreach ($docs as $doc) {
    foreach ([false, true] as $ns) {
        $p = $ns ? xml_parser_create_ns() : xml_parser_create();
        xml_set_element_handler($p, function ($p, $n, $a) { echo "S:$n ", json_encode($a), "\n"; }, function ($p, $n) { echo "E:$n\n"; });
        xml_set_character_data_handler($p, function ($p, $d) { echo "C:", bin2hex($d), "\n"; });
        xml_set_default_handler($p, function ($p, $d) { echo "D:", bin2hex($d), "\n"; });
        $r = xml_parse($p, $doc, true);
        echo json_encode(bin2hex($doc)), " ns=", (int) $ns, " r=$r err=", xml_get_error_code($p), ' ', xml_error_string(xml_get_error_code($p)),
            ' L', xml_get_current_line_number($p), ' C', xml_get_current_column_number($p), ' B', xml_get_current_byte_index($p), "\n";
    }
    // byte by byte
    $p = xml_parser_create();
    $r = 1;
    for ($i = 0; $i < strlen($doc) && $r; $i++) { $r = xml_parse($p, $doc[$i], false); }
    if ($r) { $r = xml_parse($p, '', true); }
    echo "chunked r=$r err=", xml_get_error_code($p), ' L', xml_get_current_line_number($p), ' C', xml_get_current_column_number($p), ' B', xml_get_current_byte_index($p), "\n";
    $p = xml_parser_create();
    $r = xml_parse_into_struct($p, $doc, $vals, $index);
    echo "struct r=$r ", json_encode($vals), ' ', json_encode($index), "\n";
}
"#),
    );
}

#[test]
fn libxml_errors_from_ext_xml_match_php() {
    check(
        "libxml_errors_ext_xml",
        &(PRELUDE.to_string()
            + r#"
var_dump(libxml_use_internal_errors(true));
foreach ($docs as $doc) {
    $p = xml_parser_create();
    xml_parse($p, $doc, true);
    echo json_encode(bin2hex($doc)), "\n";
    foreach (libxml_get_errors() as $e) {
        echo "  [$e->level/$e->code L$e->line C$e->column F'$e->file'] ", json_encode($e->message), "\n";
    }
    $last = libxml_get_last_error();
    echo '  last: ', $last ? json_encode([$last->level, $last->code, $last->message]) : 'false', "\n";
    libxml_clear_errors();
}
var_dump(libxml_use_internal_errors(false));
$p = xml_parser_create();
xml_parse($p, '<a><b></a>', true);
$last = libxml_get_last_error();
var_dump($last ? [$last->code, $last->line, $last->column] : $last);
libxml_clear_errors();
var_dump(libxml_get_last_error());
"#),
    );
}

#[test]
fn libxml_versions_match_php() {
    check(
        "libxml_versions",
        r#"<?php
var_dump(LIBXML_VERSION, LIBXML_DOTTED_VERSION, LIBXML_LOADED_VERSION, defined('LIBXML_NO_XXE'));
foreach (get_defined_constants(true)['libxml'] as $k => $v) { echo "$k=", var_export($v, true), "\n"; }
"#,
    );
}

#[test]
fn xml_parser_entities_match_php() {
    check(
        "xml_parser_entities",
        r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$dtd = '<!DOCTYPE r [<!ENTITY i "inner &amp; text"><!ENTITY x SYSTEM "x.ent"><!NOTATION n SYSTEM "n.bin"><!ENTITY u SYSTEM "u.bin" NDATA n>]>';
foreach (['<r>&i;&amp;&x;</r>', '<r a="&i;">&lt;&i;</r>'] as $body) {
    foreach ([0, 1, 2, 3] as $mode) {
        $p = xml_parser_create();
        if ($mode & 1) xml_set_character_data_handler($p, function ($p, $d) { echo "C:", json_encode($d), "\n"; });
        if ($mode & 2) xml_set_default_handler($p, function ($p, $d) { echo "D:", json_encode($d), "\n"; });
        xml_set_external_entity_ref_handler($p, function ($p, $names, $base, $sys, $pub) use ($mode) { echo "X:", json_encode([$names, $base, $sys, $pub]), "\n"; return $mode < 2; });
        xml_set_notation_decl_handler($p, function (...$a) { array_shift($a); echo "N:", json_encode($a), "\n"; });
        xml_set_unparsed_entity_decl_handler($p, function (...$a) { array_shift($a); echo "U:", json_encode($a), "\n"; });
        $r = xml_parse($p, $dtd . $body, true);
        echo "mode=$mode r=$r err=", xml_get_error_code($p), ' B', xml_get_current_byte_index($p), "\n";
    }
}
"#,
    );
}

#[test]
fn xmlwriter_errors_match_php() {
    check(
        "xmlwriter_errors",
        r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$w = new XMLWriter();
$w->openMemory();
var_dump($w->endElement(), $w->endAttribute(), $w->text('x'), $w->writeAttribute('a', 'b'));
var_dump($w->startDocument(), $w->startDocument(), $w->endDtd(), $w->startDtd('r'), $w->startElement('r'));
var_dump($w->endDtdElement(), $w->writeDtdAttlist('a', 'b'), $w->endCdata(), $w->endPi(), $w->endComment());
var_dump($w->startElement('e'), $w->writeAttribute('x', "\x01"), $w->text("\xFF"), $w->endDocument());
echo $w->outputMemory(), "\n";
var_dump(libxml_use_internal_errors(true));
$w = xmlwriter_open_memory();
var_dump(xmlwriter_start_document($w, '1.0', 'bogus-encoding'), xmlwriter_start_element($w, 'a'), xmlwriter_text($w, "\xE9"));
echo bin2hex(xmlwriter_output_memory($w)), "\n";
foreach (libxml_get_errors() as $e) { echo "[$e->level/$e->code] ", json_encode($e->message), "\n"; }
"#,
    );
}

/// libxml2's error handlers and error state are per thread: VMs on
/// different threads must not see each other's errors.
#[test]
fn libxml_state_is_per_thread() {
    let src = |doc: &str| {
        format!(
            r#"<?php
libxml_use_internal_errors(true);
for ($i = 0; $i < 30; $i++) {{
    $p = xml_parser_create();
    xml_parse($p, '{doc}', true);
    $w = new XMLWriter(); $w->openMemory(); $w->writeElement('e', '{doc}');
    echo count(libxml_get_errors()), ' ', xml_get_error_code($p), ' ', strlen($w->outputMemory()), "\n";
    libxml_clear_errors();
}}
"#
        )
    };
    let docs = ["<a><b></a>", "<a/>", "<a>&x;</a>", "<a x=\"1\" x=\"2\"/>"];
    let expected: Vec<Vec<u8>> = docs
        .iter()
        .map(|d| rhp_difftest::run_rhp(src(d).as_bytes()).expect("rhp runs"))
        .collect();
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let doc = docs[t % docs.len()];
            let want = expected[t % docs.len()].clone();
            let src = src(doc);
            std::thread::spawn(move || {
                for _ in 0..10 {
                    let got = rhp_difftest::run_rhp(src.as_bytes()).expect("rhp runs");
                    assert_eq!(
                        String::from_utf8_lossy(&got),
                        String::from_utf8_lossy(&want)
                    );
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
}
