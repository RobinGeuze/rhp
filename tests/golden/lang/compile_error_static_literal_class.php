<?php
// A "static" string literal before ::class is static::class: not allowed in
// a constant expression.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval('class A { const X = "Static"::class; }');
