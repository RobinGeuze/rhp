<?php
// DatePeriod's @virtual properties: declared (Reflection, property_exists),
// read through the handler, read-only, not unsettable; dynamic ones work.
$p = new DatePeriod(new DateTimeImmutable('2026-01-01 00:00:00 UTC'), new DateInterval('P1D'), 2);
var_dump($p->start->format('Y-m-d'), $p->recurrences, isset($p->end), property_exists($p, 'current'));
foreach (['start', 'include_end_date'] as $n) {
    try { $p->$n = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { unset($p->$n); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
try { $p->recurrences++; } catch (Error $e) { echo $e->getMessage(), ' #', count($e->getTrace()), "\n"; }
$r = new ReflectionProperty('DatePeriod', 'interval');
var_dump($r->isVirtual(), $r->hasHooks(), $r->getHooks(), Reflection::getModifierNames($r->getModifiers()), (string) $r->getType());
var_dump(array_map(fn ($p) => $p->name, (new ReflectionClass('DatePeriod'))->getProperties()));
var_dump(method_exists($p, '__get'), array_keys((array) $p));
$q = (new ReflectionClass('DatePeriod'))->newInstanceWithoutConstructor(); var_dump(isset($q->start));
