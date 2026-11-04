//! ext/date output that comes from the tz database's own listing: its
//! version, the identifier lists, a zone's location and transitions. rhp
//! embeds php's internal database; a php built against the system's tzdata
//! (CI's reports "0.system") lists them differently, so these run only when
//! php's database version is rhp's. Skips without `php`.

use rhp_difftest::{diff_source, php_available, run_php, run_rhp};

fn check(name: &str, src: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    let src_version = b"<?php echo timezone_version_get();";
    let version =
        |out: Result<Vec<u8>, String>| out.map(|o| String::from_utf8_lossy(&o).into_owned());
    let (php, rhp) = (version(run_php(src_version)), version(run_rhp(src_version)));
    if php != rhp {
        eprintln!("[{name}] php's tz database is {php:?}, rhp's {rhp:?} — skipping");
        return;
    }
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

#[test]
fn identifier_lists_match_php() {
    check(
        "identifiers",
        r#"<?php
var_dump(timezone_version_get());
var_dump(count(timezone_identifiers_list()), count(timezone_identifiers_list(DateTimeZone::ALL_WITH_BC)));
var_dump(timezone_identifiers_list(DateTimeZone::PER_COUNTRY, 'NL'), timezone_identifiers_list(DateTimeZone::UTC));
var_dump(count(timezone_identifiers_list(DateTimeZone::EUROPE | DateTimeZone::ARCTIC)));
foreach ([DateTimeZone::ALL, DateTimeZone::ALL_WITH_BC, DateTimeZone::AFRICA | DateTimeZone::PACIFIC] as $g) {
    echo $g, ' ', md5(implode(',', DateTimeZone::listIdentifiers($g))), "\n";
}
echo json_encode(DateTimeZone::listIdentifiers(DateTimeZone::PER_COUNTRY, 'US')), "\n";
"#,
    );
}

#[test]
fn locations_and_transitions_match_php() {
    check(
        "transitions",
        r#"<?php
foreach (['Europe/Amsterdam', 'America/New_York', 'Australia/Lord_Howe', 'Asia/Kolkata', 'UTC'] as $id) {
    $tz = new DateTimeZone($id);
    echo $id, ' ', json_encode($tz->getLocation()), "\n";
    $t = $tz->getTransitions();
    echo count($t), ' ', json_encode(array_slice($t, 0, 3)), ' ', json_encode(array_slice($t, -2)), "\n";
    echo json_encode($tz->getTransitions(-2000000000, 2000000000)), "\n";
}
echo json_encode(timezone_location_get(new DateTimeZone('Europe/Paris'))), "\n";
echo json_encode(timezone_transitions_get(new DateTimeZone('Asia/Kathmandu'), 0, 600000000)), "\n";
"#,
    );
}
