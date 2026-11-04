<?php
// php-src ext/opcache/tests/opt/gh14873.phpt without its opcode dump
// (tests/phpt/ignored.txt): GH-14873 (PHP 8.4 min function fails on typed
// integer)

function testTrim1(string $value): string {
	$value = trim($value);
	return $value;
}

function testMin2First(int $value): int {
	$value = min($value, 100);
	return $value;
}

function testMin2Second(int $value): int {
	$value = min(100, $value);
	return $value;
}

function testMin2_TMP(int $value): int {
	$value = min($value + 1, 100);
	return $value;
}

function testStrstr3First(string $value): string {
	$value = strstr($value, "needle", false);
	return $value;
}

function testStrstr3Second(string $value): string {
	$value = strstr("needles", $value, false);
	return $value;
}

function testStrstr3Third(bool $value): string {
	$value = strstr("needles", "needle", $value);
	return $value;
}

var_dump(testTrim1(" boo "));
var_dump(testMin2First(5));
var_dump(testMin2Second(5));
var_dump(testMin2_TMP(5));
var_dump(testStrstr3First("needles"));
var_dump(testStrstr3Second("needle"));
var_dump(testStrstr3Third(false));

?>
