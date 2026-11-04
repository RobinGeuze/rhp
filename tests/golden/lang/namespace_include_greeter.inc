<?php
namespace Acme\Hello;
class Greeter {
    public function __construct(private string $who) {}
    public function greet(): string { return "Hello, {$this->who}!"; }
}
function strlen($s) { return "SHADOWED"; }
