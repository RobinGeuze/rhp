<?php
// Closure's stub constructor is private: a callable or a call reaching it
// is refused as php's zend_std_get_method / zend_bad_method_call do.
$c = function () {};
foreach ([[$c, '__construct'], [$c, '__CONSTRUCT'], [$c, 'nope']] as $cb) {
    try { call_user_func($cb); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    var_dump(is_callable($cb));
}
try { $c->__construct(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
class K { function f($c) { try { $c->__construct(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } } }
(new K)->f($c);
