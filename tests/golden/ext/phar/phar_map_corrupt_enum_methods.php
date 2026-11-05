<?php
// Phar::mapPhar() on a file whose data after __HALT_COMPILER() is not a phar
// manifest reports php's own corruption message; get_class_methods() of an
// enum lists its built-in cases/from/tryFrom after its own methods.
foreach ([null, 'x.phar'] as $alias) {
    try { var_dump($alias === null ? Phar::mapPhar() : Phar::mapPhar($alias)); }
    catch (PharException $e) { echo get_class($e), ': ', str_replace(__FILE__, 'FILE', $e->getMessage()), "\n"; }
}
var_dump(Phar::running(), Phar::running(false));

enum Backed: string { case X = 'x'; function own() {} }
enum Pure { case A; private function hidden() {} }
var_dump(get_class_methods('Backed'), get_class_methods('Pure'), get_class_methods(Backed::X));
__HALT_COMPILER();junk
