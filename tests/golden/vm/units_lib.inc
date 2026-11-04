<?php
// Included once: its closures and nested thunks live past index 0 of the
// engine's tables.
$fromLib = function () { return 'lib closure'; };

#[Attribute]
class Tag
{
    public function __construct(public Closure $make) {}
}

function withDefault(Closure $c = static function () { return 'default thunk'; })
{
    return $c();
}

#[Tag(static function () { return 'attribute thunk'; })]
function tagged() {}

class Consts
{
    const MAKE = static function () { return 'constant thunk'; };
    public $p = [1];

    public function counter()
    {
        return function () { static $k = 0; return ++$k; };
    }
}

trait Maker
{
    public static $label = PHP_EOL === "\n" ? 'trait static' : '?';
    #[Tag(static function () { return 'trait property attribute'; })]
    public $tagged = 1;
    #[Tag(static function () { return 'trait constant attribute'; })]
    const TAGGED = 1;

    public function make()
    {
        return fn() => 'trait closure';
    }
}

function declareNested()
{
    if (!class_exists('Nested', false)) {
        class Nested { const N = 'nested class'; }
    }
}
