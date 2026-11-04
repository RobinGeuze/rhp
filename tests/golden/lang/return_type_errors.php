<?php
function p(Throwable $e) { echo str_replace(__FILE__, basename(__FILE__), $e->getMessage()), "\n"; }
class T { function m(): static { return new stdClass; } function s(): self { return 1; } static function q(): int { return "x"; } }
try { (new T)->m(); } catch (TypeError $e) { p($e); }
try { (new T)->s(); } catch (TypeError $e) { p($e); }
try { T::q(); } catch (TypeError $e) { p($e); }
function f(): ?int { return []; }
try { f(); } catch (TypeError $e) { p($e); }
$c = function(): int { return "a"; };
try { $c(); } catch (TypeError $e) { p($e); }
class U extends T {}
try { (new U)->m(); } catch (TypeError $e) { p($e); }
function v(): void { return; }
function n(): never { }
try { n(); } catch (TypeError $e) { p($e); }
class P { function a(self $x) {} }
class Q extends P { function c(parent $p, int|string|null $z = 0) {} function d(): ?static { return 5; } function e(): iterable { return 1; } function f(): int|float|false { return []; } function g(): true|null { return 1; } }
try { (new Q)->a(new stdClass); } catch (TypeError $e) { p($e); }
try { (new Q)->c(new stdClass); } catch (TypeError $e) { p($e); }
try { (new Q)->c(new P, []); } catch (TypeError $e) { p($e); }
foreach (['d','e','f','g'] as $m) try { (new Q)->$m(); } catch (TypeError $e) { p($e); }
class W { function cl() { return function(): int { return "a"; }; } }
try { ((new W)->cl())(); } catch (TypeError $e) { p($e); }
function f1(): ?int {} try { f1(); } catch (TypeError $e) { p($e); }
function f2(): mixed { if (0) return 1; } try { f2(); } catch (TypeError $e) { p($e); }
function f3(): void { return; } var_dump(f3());
function f4(): int { try { return "x"; } finally { echo "fin\n"; } } try { f4(); } catch (TypeError $e) { p($e); }
function f5(): int { try { return "12"; } finally { echo "fin\n"; } } var_dump(f5());
function gen(): Generator { yield 1; return 5; } $g = gen(); foreach ($g as $x) echo $x, "\n"; echo $g->getReturn(), "\n";
function gen2(): iterable { yield 1; } foreach (gen2() as $x) echo $x, "\n";
class S { static function make(): static { return new static; } } class S2 extends S {} echo get_class(S2::make()), "\n";
