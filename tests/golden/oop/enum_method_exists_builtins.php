<?php
enum E: string { case A='a'; } enum U { case X; }
foreach (['cases','from','tryFrom','TRYFROM','foo'] as $m) var_dump(method_exists(E::class,$m), method_exists(U::class,$m), method_exists(E::A,$m), method_exists(U::X, $m));
var_dump(method_exists('stdClass','cases'));
