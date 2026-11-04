<?php
// exit() in a destructor run while an exception unwinds ends the request:
// the exception is dropped and the caller's finally never runs (php 8.5.12
// also no longer attaches it as UnwindExit::$previous).
function f(){ $o = new class{function __destruct(){echo "dtor\n"; exit("bye\n");}}; throw new Exception("x"); }
try { f(); } finally { echo "finally\n"; }
