<?php
// No output may happen before the header calls (CLI sends headers on first output).
$log = [];
$log[] = headers_sent();
$log[] = http_response_code();
header('X-Test: 1');
$log[] = headers_list();
$log[] = http_response_code(404);
$log[] = http_response_code();
header('Location: /x');
$log[] = http_response_code();
header('X-Code: 1', true, 201);
$log[] = http_response_code();
header('Location: /y');
$log[] = http_response_code();
header('HTTP/1.1 500 Internal Server Error');
header_remove('X-Test');
header_remove();
$log[] = headers_sent();
var_dump($log);
var_dump(headers_sent());
