<?php
// The internal Directory class: only dir() creates it, readonly properties,
// no dynamic properties, not cloneable, methods take no arguments; its
// engine helper is not a visible function.
function t($f) {
    try { $r = $f(); echo "ok: ", var_export($r, true), "\n"; }
    catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
var_dump(function_exists('__directory_op'), is_callable('__directory_op'));
$r = new ReflectionClass('Directory');
foreach ($r->getMethods() as $m) echo $m->getName(), "\n";
var_dump(method_exists('Directory', '__construct'), $r->isFinal());
foreach ($r->getProperties() as $p) var_dump($p->getName());

$base = sys_get_temp_dir() . "/rhp_dcls_" . bin2hex(random_bytes(6));
mkdir($base);
touch("$base/a");
$d = dir($base);
var_dump($d instanceof Directory, $d->path === $base, is_resource($d->handle));
t(function () use ($d) { $d->path = 'x'; });
t(function () use ($d) { $d->handle = null; });
t(function () use ($d) { $d->foo = 1; });
t(function () use ($d) { unset($d->path); });
t(function () use ($d) { return clone $d; });
t(function () use ($d) { return $d->read(1); });
t(function () use ($d) { $d->rewind(1, 2); });
t(function () use ($d) { $d->close(1); });
t(function () { return new Directory(); });
t(function () { return new Directory('x'); });
$names = [];
while (($e = $d->read()) !== false) $names[] = $e;
sort($names);
var_dump($names);
$d->rewind();
var_dump($d->read() !== false);
$d->close();
t(function () use ($d) { return $d->read(); });
var_dump($d->path === $base);
unlink("$base/a");
rmdir($base);
