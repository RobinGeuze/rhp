<?php
// A namespaced class from a required file, used from two namespaces
// (imported and fully qualified); a namespaced function shadowing a global
// one, `\name` forcing the global, and the fall-back to the global.
namespace Acme\Hello;
require __DIR__ . '/namespace_include_greeter.inc';
$g = new Greeter("World");
echo $g->greet(), "\n";
echo strlen("abc"), "\n";   // namespaced function wins
echo \strlen("abc"), "\n";  // forced global

namespace Other;
require_once __DIR__ . '/namespace_include_greeter.inc';
use Acme\Hello\Greeter;
$g2 = new Greeter("there");
echo $g2->greet(), "\n";
echo strlen("abcd"), "\n";  // global fall-back
