<?php
function a() { throw new RuntimeException("from a"); }
function b() { a(); }
try {
    b();
} catch (RuntimeException $e) {
    echo $e->getMessage(), "|", $e->getLine(), "\n";
}
try {
    throw new LogicException("direct");
} catch (Exception $e) {
    echo get_class($e), "|", $e->getLine(), "\n";
}
$ex = new Exception("made");
echo $ex->getLine(), "\n";
var_dump($ex->getTrace());
echo $ex->getTraceAsString(), "\n";
class Custom extends Exception {}
try { throw new Custom("c", 42); }
catch (Custom $c) { echo $c->getLine(), "|", $c->getCode(), "\n"; }
