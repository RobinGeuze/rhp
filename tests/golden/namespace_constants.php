<?php
// Namespaced constants: `use const`, fully qualified, the global
// fall-back, and define() of a namespaced name.
namespace Lib;
const VERSION = '1.2.3';
namespace App;
use const Lib\VERSION;
echo VERSION, "\n";
echo \Lib\VERSION, "\n";
echo PHP_EOL === "\n" ? "eol-ok\n" : "eol-no\n";
define('App\\DEFINED', 42);
echo DEFINED, "\n";
