<?php
// The deprecations are compile-time: silence them before the include.
error_reporting(E_ALL & ~E_DEPRECATED);
include __DIR__ . "/backtick_namespace.inc";
