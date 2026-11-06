<?php
// WeakMap and WeakReference behaviour the phpt files leave open.
set_error_handler(function ($no, $msg) { echo "error: $msg\n"; return true; });

$m = new WeakMap;
$a = new stdClass;
$b = new stdClass;
$c = function () {};

// Closures are objects: keys and referents.
$m[$c] = 'closure';
$wc = WeakReference::create($c);
var_dump(count($m), $wc->get() === $c);
unset($c);
var_dump(count($m), $wc->get());

// Compound assignment, ??, isset/empty on null and falsy values.
$m[$a] = 'x';
$m[$a] .= 'y';
var_dump($m[$a]);
$m[$b] = null;
var_dump(isset($m[$b]), empty($m[$b]), $m[$b] ?? 'dflt', isset($m[new stdClass]));
$m[$b] = 0;
var_dump(isset($m[$b]), empty($m[$b]));

// Nested writes, and the notice of an unset below a WeakMap element.
$m[$a] = ['k' => [1]];
$m[$a]['k'][] = 2;
$m[$a]['n'] = 3;
unset($m[$a]['k'][0]);
var_dump($m[$a]);
try {
    $m[new stdClass]['x'] = 1;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// Entries added while iterating are visited; removed ones are skipped.
$keys = [new stdClass, new stdClass, new stdClass];
$m2 = new WeakMap;
foreach ($keys as $i => $k) {
    $m2[$k] = $i;
}
$extra = new stdClass;
foreach ($m2 as $k => $v) {
    echo "visit $v\n";
    if ($v === 0) {
        unset($m2[$keys[1]]);
        $m2[$extra] = 9;
    }
}

// A clone has its own entries; both forget a freed key.
$m3 = clone $m2;
unset($m3[$keys[0]]);
var_dump(count($m2), count($m3));
unset($keys);
var_dump(count($m2), count($m3));

// The iterator object getIterator() returns.
$it = $m2->getIterator();
var_dump(get_class($it));
foreach ($it as $k => $v) {
    var_dump($v);
}
try {
    clone $it;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// A WeakMap as a key of another, and a map freed before its keys.
$outer = new WeakMap;
$inner = new WeakMap;
$outer[$inner] = 'inner';
$inner[$a] = 1;
unset($inner);
var_dump(count($outer));

// WeakReference refusals.
try {
    new WeakReference;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
$r = WeakReference::create($a);
try {
    clone $r;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    serialize($r);
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}
var_dump((array) $r, (array) $m2, get_object_vars($m2));
print_r($r);
echo "\n";
print_r($m2);
echo "\n";
// The referent's handle is reused after it is freed.
unset($a, $m, $m3);
var_dump($r->get());
$n = new stdClass;
var_dump($r->get(), WeakReference::create($n) === $r);
