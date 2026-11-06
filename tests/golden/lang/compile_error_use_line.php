<?php
// A use conflict is reported at the statement's first name.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval('namespace N;
class FooBar {}
use A\{
    Baz,
    FooBar
};');
