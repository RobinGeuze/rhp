<?php
// A literal left side of && / || that decides the result leaves the right
// side uncompiled, but php's scanner still reports what it reads there.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval('var_dump(false && (integer) "1");
var_dump(true || (boolean) $x);
var_dump(0 and "\400" . (double) 1);
var_dump(\'1\' or "a\401$y" . <<<EOT
  \402 $z
  EOT . <<<\'NOW\'
  \403
  NOW);
var_dump(null && (binary) `x\404`);
var_dump("\405" || f());
var_dump((false && f()) || (integer) "1");
var_dump(!true && new class { function f() { parent::x(); } });
var_dump((true && false) && (boolean) i());');
