<?php
// An array met again inside itself is an Error, checked before the sizes.
$a = [1]; $a[] = &$a;
$l = [1, [1, 2, 3]];
try { var_dump($l == $a); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { var_dump($a == $l); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$b = [1, 2]; $b[] = &$b;
try { var_dump($b == [1, 2, [5]]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { var_dump($b == [1, 2, [5, 6, 7, 8]]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
