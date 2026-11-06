<?php
// Where php's TICKS land: echo/global/static/unset lists tick per item, and
// no second tick right after one (an if without else, a last catch).
declare(ticks=1);
function t() { echo "T "; }
register_tick_function('t');
echo "\nif:"; $a = 1;
if ($a) { $b = 2; }
if (!$a) { $b = 1; } elseif ($a) { $b = 2; }
if ($a) {}
echo "\necho:"; echo "a", "b", "c";
echo "\nglobal:";
function g() { global $x, $y; static $s = 1, $r = 2; } g();
echo "\nunset:"; $p = 1; $q = 2; unset($p, $q);
echo "\ntry:"; try { $a = 1; } catch (Exception $e) { $a = 2; }
try { throw new Exception; } catch (TypeError $e) { $a = 2; } catch (Exception $e) { $a = 3; }
try { $a = 1; } finally { $a = 2; }
echo "\nloop:"; $i = 0; while ($i < 2) { $i++; if ($i) { $k = 1; } }
echo "\n";
