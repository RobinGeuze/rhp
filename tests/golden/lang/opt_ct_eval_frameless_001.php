<?php
// php-src ext/opcache/tests/ct_eval_frameless_001.phpt without its opcode dump
// (tests/phpt/ignored.txt): Test ct eval of frameless function
// basename(): the full path differs per machine. Two levels up from
// tests/golden/lang is tests/.
echo basename(dirname(__DIR__, 2));
?>
