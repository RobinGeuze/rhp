<?php
// php-src ext/opcache/tests/ct_eval_frameless_001.phpt without its opcode dump
// (tests/phpt/ignored.txt): Test ct eval of frameless function
// basename(): the full path differs per machine.
echo basename(dirname(__DIR__));
?>
