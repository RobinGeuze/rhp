<?php
function f1() { return $http_response_header ?? null; }
function f2() { return isset($http_response_header) ? 1 : 0; }
function f3() { $http_response_header ??= []; return $http_response_header; }
function f4() { return @$http_response_header; }
function f5() { return "x $http_response_header y"; }
function f6() { return $http_response_header[0] ?? null; }
function f7() { return empty($http_response_header); }
function f8() { $x = &$http_response_header; return $http_response_header; }
function f9() { foreach ([1] as $http_response_header) {} return $http_response_header; }
function f10() { [$http_response_header] = [1]; return $http_response_header; }
function f11() { return compact('http_response_header'); }
function f12() { $f = fn() => $http_response_header; return $f; }
function f13() { global $http_response_header; return $http_response_header; }
function f14() { static $http_response_header; return $http_response_header; }
function f15() { return $http_response_header?->x; }
function f16() { return $http_response_header::class; }
function f17() { return array_key_exists(0, $http_response_header); }
function f18() { unset($http_response_header); return 1; }
function f19() { $http_response_header++; return $http_response_header; }
function f20() { return match(true) { default => $http_response_header }; }
function f21() { return is_null($http_response_header ?? null) || $http_response_header === null; }
function f22() { try {} catch (Exception $http_response_header) {} return $http_response_header; }
function f23() { list('a' => $http_response_header) = ['a' => 1]; return $http_response_header; }
function f24() { return $http_response_header->prop ?? 1; }
function f25() { return sort($http_response_header); }
function f26() { return isset($http_response_header[0]) && $http_response_header[0]; }
function f27($a = 1) { return ${'http_response_header'}; }
function f28() { return (string) $http_response_header; }
function f29() { foreach ([1] as &$http_response_header) {} return 1; }
function f30() { foreach ($http_response_header as &$v) {} return $http_response_header; }
function &f31() { return $http_response_header; }
function f32() { $a = [&$http_response_header]; return $http_response_header; }
function f33() { [$a] = $http_response_header; return $a; }
function f34() { return new ArrayObject($http_response_header[0]); }
function f35() { return explode(' ', $http_response_header[0]); }
function f36() { return undefined_fn($http_response_header[0], $http_response_header); }
function f37() { $http_response_header[0] ??= 1; return $http_response_header; }
function f38() { return $http_response_header?->a->b ?? 2; }
function f39() { return preg_match('/x/', $http_response_header[0], $m); }
function f40() { return empty($http_response_header . ''); }
function user_byref(&$x) {}
function user_byval($x) {}
function f41() { user_byref($http_response_header[0]); return 1; }
function f42() { user_byval($http_response_header[0]); return 1; }
function f43() { later_fn($http_response_header[0]); return 1; }
function later_fn($x) {}
function f44() { return strlen(string: $http_response_header[0]); }
function f45() { return array_push(...$http_response_header); }
