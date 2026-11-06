<?php
// assert()'s description under strict_types: scalars are refused.
declare(strict_types=1);
function t($f) { try { $f(); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
t(fn() => assert(false, 5));
t(fn() => assert(false, 1.5));
t(fn() => assert(false, true));
t(fn() => assert(false, "s"));
t(fn() => assert(true, 5));
t(fn() => call_user_func("assert", false, 5));
$f = "assert"; t(fn() => $f(false, 5));
