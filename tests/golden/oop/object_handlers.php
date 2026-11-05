<?php
// Internal classes' object handlers: do_operation, cast_object, compare,
// clone_obj, and the class flags (not serializable, no dynamic properties).
function t(callable $f) {
    try {
        $r = $f();
        echo is_object($r) ? get_class($r) . ' ' . (string) $r : var_export($r, true), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

echo "-- GMP do_operation\n";
$a = gmp_init(5);
foreach ([
    fn() => $a + 3, fn() => 3 - $a, fn() => $a * $a, fn() => $a / 2, fn() => -7 / $a,
    fn() => $a % 3, fn() => gmp_init(-7) % 3, fn() => $a ** 3, fn() => $a << 4,
    fn() => gmp_init(-9) >> 1, fn() => $a | 2, fn() => $a & 4, fn() => $a ^ 1, fn() => ~$a,
    fn() => -$a, fn() => $a + "0x10", fn() => $a + true,
] as $f) {
    t($f);
}
$b = $a;
$b += 10;
$b *= 2;
echo $b, " ", $a, "\n";
$c = $a;
$c++;
$c--;
$c--;
echo $c, "\n";

echo "-- GMP failures\n";
foreach ([
    fn() => $a + "abc", fn() => $a + [], fn() => $a + new stdClass, fn() => $a / 0,
    fn() => $a % 0, fn() => $a << -1, fn() => $a ** -1, fn() => $a ** [], fn() => [] << $a, fn() => "abc" << $a,
    fn() => $a . "x", fn() => $a + null,
] as $f) {
    t($f);
}

echo "-- GMP cast_object and compare\n";
var_dump((string) $a, (int) $a, (float) $a, (bool) gmp_init(0), (bool) $a);
var_dump(intval(gmp_init(-12)), floatval(gmp_init(8)), boolval(gmp_init(0)));
var_dump($a == 5, $a < 6, $a <=> gmp_init(7), 5 == $a, "5" == $a, $a == 5.0);
t(fn() => $a == "x");
t(fn() => $a == []);

echo "-- object increments\n";
$o = new stdClass;
t(function () use ($o) { $o++; return $o; });
t(function () use ($o) { $o--; return $o; });

echo "-- clone_obj\n";
class MyException extends Exception {}
class MyIterator extends IteratorIterator {}
foreach ([
    fn() => new Exception("e"), fn() => new MyException, fn() => new ReflectionClass('stdClass'),
    fn() => new ReflectionMethod('ArrayObject', 'count'), fn() => xml_parser_create(),
    fn() => new XMLWriter, fn() => new XMLReader, fn() => new MyIterator(new ArrayIterator([])),
    fn() => dir(__DIR__), fn() => curl_multi_init(), fn() => curl_share_init(),
    fn() => new ArrayObject([1]), fn() => new WeakMap, fn() => new SplObjectStorage,
] as $f) {
    $obj = $f();
    try {
        $copy = clone $obj;
        echo "cloned ", get_class($copy), "\n";
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
$h = curl_init("http://localhost/");
$h2 = clone $h;
var_dump(curl_getinfo($h2, CURLINFO_EFFECTIVE_URL), (int) $h === spl_object_id($h));

echo "-- not serializable\n";
foreach ([
    fn() => serialize(xml_parser_create()), fn() => serialize(new ReflectionClass('stdClass')),
    fn() => serialize(dir(__DIR__)), fn() => serialize(new WeakMap), fn() => serialize(curl_init()),
    fn() => serialize(simplexml_load_string('<a>x</a>')), fn() => serialize(new SplFileInfo(__FILE__)),
    fn() => serialize((function () { yield 1; })()), fn() => unserialize('O:9:"XMLParser":0:{}'),
    fn() => unserialize('O:16:"SimpleXMLElement":0:{}'), fn() => unserialize('O:9:"Directory":0:{}'),
] as $f) {
    t($f);
}

echo "-- dynamic properties\n";
foreach ([
    function () { $p = xml_parser_create(); $p->x = 1; },
    function () { $e = new DateError("d"); $e->x = 1; },
    function () { $c = curl_init(); $c->x = 1; },
    function () { $o = new stdClass; $o->x = 1; return $o->x; },
] as $f) {
    t($f);
}

echo "-- compare\n";
$p1 = xml_parser_create();
$p2 = xml_parser_create();
var_dump($p1 == $p2, $p1 == $p1, $p1 < $p2, $p1 > $p2, $p1 == 1, $p1 == true);
$d1 = dir(__DIR__);
$d2 = dir(__DIR__);
var_dump($d1 == $d2, $d1 == $d1);
$c1 = curl_init();
var_dump($c1 == $c1, $c1 == curl_init());
t(fn() => $a == $c1);
var_dump(new DateTime('2020-01-01') == new DateTimeImmutable('2020-01-01'));
var_dump(new DateTime('2020-01-01') < new DateTime('2021-01-01'));
var_dump(new DateTime('2020-01-01') == new stdClass);

// Diagnostics without the fixture's path.
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});

echo "-- operand order (convert_op1_op2_long, concat_function)\n";
$arr = [];
$s = 'abc';
$fl = 2.5;
$o = new stdClass;
foreach ([
    fn() => $arr | $a, fn() => $arr % $a, fn() => $o % $a, fn() => $o ^ $a, fn() => $o << $a,
    fn() => $s & $a, fn() => $fl | $a, fn() => $arr . $a, fn() => $o + 1, fn() => [] + $o,
    fn() => (function () {}) * 2, fn() => -$o, fn() => ~$o,
] as $f) {
    t($f);
}
try {
    "5x" << $a;
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), " / ", get_class($e->getPrevious()), "\n";
}

echo "-- GMP edges\n";
var_dump((float) gmp_init("18014398509481987"), floatval(gmp_init("-18014398509481987")));
foreach ([
    fn() => gmp_init(7) >> 5000000000, fn() => gmp_init(-7) >> PHP_INT_MAX,
    fn() => gmp_init(1) ** 5000000000, fn() => gmp_init(0) ** 5000000000,
    fn() => gmp_init(-1) ** 5000000001, fn() => gmp_init(0) << 5000000000,
] as $f) {
    t($f);
}
t(fn() => array_map(fn($x) => $x + 1.5, [$a])[0]);
$cl = function () {};
var_dump($cl == $a, $cl < $a);

echo "-- conversions\n";
var_dump((int) $o, intval($o), (float) curl_init());
$e = simplexml_load_string('<a><b/></a>')->c;
$x = simplexml_load_string('<a>5</a>');
var_dump(count(array_filter([gmp_init(0), $e, 1, $x])), array_sum([$x, 1, [], 'abc']));
$h = new stdClass;
$h->g = gmp_init(0);
var_dump(empty($h->g));

echo "-- flags\n";
t(fn() => serialize(new class {}));
foreach (['XMLParser', 'GMP', 'Closure', 'Directory'] as $cls) {
    t(fn() => get_class((new ReflectionClass($cls))->newInstanceWithoutConstructor()));
}
$c2 = curl_init();
t(function () use ($c2) { $c2->z[] = 1; });
t(function () use ($c2) { $c2->q++; });
t(function () use ($c2) { $r = &$c2->w; });
