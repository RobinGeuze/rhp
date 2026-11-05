<?php
// The generator return type check reports the declaration's line.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval('class A
{
    #[Attr]
    public function m()
        : int
    {
        $x = 1;
        yield $x;
    }
}');
