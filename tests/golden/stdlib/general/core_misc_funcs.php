<?php
// iptcparse/iptcembed, get_resources/get_resource_id, get_error_handler,
// method_exists on inherited privates, class_alias in get_declared_*(),
// ini_parse_quantity, json_encode of a JsonSerializable enum, exit()'s
// argument checks.

$jpeg = __DIR__ . '/core_misc_funcs.jpg';
file_put_contents($jpeg, "\xff\xd8\xff\xe0\x00\x04ab\xff\xdb\x00\x03c\xff\xda\x00\x02DATA");
$iptc = "\x1c\x02\x05\x00\x05Title\x1c\x02\x19\x00\x03one\x1c\x02\x19\x00\x03two";
$out = iptcembed($iptc, $jpeg);
var_dump(bin2hex($out));
var_dump(iptcparse($iptc));
var_dump(iptcparse("no iptc here"));
var_dump(iptcembed("odd", $jpeg, 0) === iptcembed("odd\0", $jpeg));
file_put_contents($jpeg, "not a jpeg");
var_dump(iptcembed($iptc, $jpeg));
unlink($jpeg);

$f = fopen('php://memory', 'r+');
$res = get_resources();
var_dump(isset($res[(int) $f]), get_resource_id($f) === (int) $f);
var_dump(count(get_resources('stream')) >= 4);
fclose($f);
var_dump(get_resource_id($f) === (int) $f, array_key_exists((int) $f, get_resources('Unknown')));
try {
    get_resources('nope');
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

var_dump(get_error_handler(), get_exception_handler());
set_error_handler('var_dump', E_WARNING);
var_dump(get_error_handler());
restore_error_handler();
var_dump(get_error_handler());

class Base { private function hidden() {} private static function shidden() {} }
class Kid extends Base { private function own() {} }
var_dump(method_exists('Kid', 'hidden'), method_exists(new Kid, 'hidden'),
    method_exists('Kid', 'shidden'), method_exists('Kid', 'own'), method_exists('Base', 'hidden'));

interface I {}
trait Tr {}
class_alias('Kid', 'KidAlias');
class_alias('I', 'IAlias');
class_alias('Tr', 'TrAlias');
$tail = fn ($a) => array_slice($a, -2);
var_dump($tail(get_declared_classes()), $tail(get_declared_interfaces()), $tail(get_declared_traits()));

foreach (['1k', '0x10', ' 2M ', '1g', '-1', '12q', 'abc', '0b', '0z1'] as $q) {
    error_clear_last();
    var_dump(@ini_parse_quantity($q), error_get_last()['message'] ?? null);
}

enum Plain implements JsonSerializable {
    case A;
    public function jsonSerialize(): mixed { return 'plain-' . $this->name; }
}
enum Backed: int implements JsonSerializable {
    case B = 2;
    public function jsonSerialize(): mixed { return ['b' => $this->value]; }
}
var_dump(json_encode([Plain::A, Backed::B]));

var_dump(M_SQRT3, M_EULER, M_LNPI, INI_ALL, EXTR_REFS, IMAGETYPE_WEBP, DNS_ALL, PSFS_PASS_ON);

foreach ([[], new stdClass, "bye\n"] as $v) {
    try {
        exit($v);
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
echo "not reached\n";
