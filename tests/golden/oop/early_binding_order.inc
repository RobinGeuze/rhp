<?php
// php-parser's TokenPolyfill pattern: the top-level declaration implements
// Stringable (via __toString), so it is not early-bound and the conditional
// one wins.
namespace EarlyBind;

if (true) {
    class TokenPoly extends \ArrayObject {}
    return;
}
class TokenPoly
{
    public function __toString(): string { return ''; }
}
