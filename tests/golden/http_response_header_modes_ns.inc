<?php
namespace App;
use function strtoupper as up;
function a() { return explode(' ', $http_response_header[0]); }
function b() { return \explode(' ', $http_response_header[0]); }
function c() { return sort($http_response_header); }
function d() { return count($http_response_header); }
function e() { return isset($http_response_header) ? $http_response_header : []; }
class K { function m() { return strtolower($http_response_header[0] ?? ''); } }
function g() { return strpos($http_response_header[0], '200'); }
function h() { return str_contains($http_response_header[0], ' 200 '); }
function i() { return trim($http_response_header); }
function j() { return trim(substr($http_response_header[0], 0, 3)); }
function k() { return preg_match('{^HTTP/\S+ (\d+)}', $http_response_header[0], $m); }
function l() { return in_array('x', $http_response_header, true); }
function m() { return up($http_response_header[0]); }
function n() { return implode(', ', $http_response_header); }
function o() { return str_replace('a', 'b', $http_response_header[0]); }
function p() { return trim($http_response_header[0], ' ', 'extra'); }
function q() { return namespace\trim($http_response_header[0]); }
function r() { return trim(string: $http_response_header[0]); }
