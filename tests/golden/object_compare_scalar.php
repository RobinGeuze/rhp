<?php
class T { function __toString(): string { return 'abc'; } }
class A { public $x = 1; }
var_dump(new T == 'abc', in_array('abc', [new T]), new T < 'abd', 'abc' == new T, new T == new T);
var_dump(array_search('abc', ['x', new T]), array_keys([new T, 'q', new T], 'abc'));
var_dump(@(new A == 1));
$e = error_get_last(); echo $e['type'], ' ', $e['message'], "\n";
var_dump(@(new A == 1.5), @(new A < 2), @(2 > new A));
echo error_get_last()['message'], "\n";
error_clear_last(); var_dump(error_get_last());
var_dump(new A < [], [] < new A, new A <=> [], [] <=> new A, new A == [], new A == 'x', new A <=> 'x', 'x' <=> new A);
var_dump(new A == true, new A == false, new A == null, null < new A);
$f = fn() => 1;
var_dump(@($f == 2), $f == $f, $f == fn() => 1, $f == 'x');
echo error_get_last()['message'], "\n";
// GMP's own compare handler
var_dump(gmp_init(5) == 5, gmp_init(7) > 3, gmp_init(0) == false, gmp_init(10) == "10", gmp_init(10) < gmp_init(9), gmp_init(3) <=> "0x10", 4 <=> gmp_init(4));
try { var_dump(gmp_init(1) == "abc"); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { var_dump(gmp_init(1) == null); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { var_dump(gmp_init(1) == [1]); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// DateTimeInterface compares across DateTime / DateTimeImmutable
var_dump(new DateTime('2020-01-01') == new DateTimeImmutable('2020-01-01'), new DateTime('2020-01-01') < new DateTimeImmutable('2020-01-02'), new DateTimeImmutable('2021-01-01') <=> new DateTime('2020-01-01'));
// recursion
class R { public $x; }
$r1 = new R; $r1->x = $r1; $r2 = new R; $r2->x = $r2;
try { var_dump($r1 == $r2); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { var_dump($r1 < $r2); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { var_dump(in_array($r1, [$r2])); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump($r1 == $r1);
echo "still running\n";
