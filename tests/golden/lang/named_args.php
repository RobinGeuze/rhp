<?php
function greet(string $name, string $greeting = "Hello", string $punct = "!") {
    return "$greeting, $name$punct";
}
echo greet(name: "Bob"), "\n";
echo greet(name: "Al", greeting: "Hi"), "\n";
echo greet("Cy", punct: "?"), "\n";
echo greet(greeting: "Hey", name: "Dee"), "\n";

function box(int $w, int $h = 10, int $d = 1) { return $w * $h * $d; }
echo box(5), " ", box(5, d: 3), " ", box(w: 2, h: 3, d: 4), "\n";

function describe(string $item, int $count = 0, bool $urgent = false) {
    return ($urgent ? "URGENT " : "") . "$count $item";
}
echo describe(item: "box", urgent: true), "\n";
echo describe("widget", count: 5), "\n";

$qty = 3;
function order(string $type, int $qty = 1, float $price = 9.99) {
    return "$qty $type @ $price";
}
echo order(type: "apple", qty: $qty * 2), "\n";
echo order("pear", price: 1.50), "\n";
