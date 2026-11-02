<?php
header("X-A: 1");
header("x-a: 2", false);
header("X-B: trailing   \t ");
header("X-C: first");
header("X-C: second");
header("Content-Type: application/json");
header("Content-Type: text/plain");
header("X-Empty:");
header("");
header("Bad\nHeader: x");
header("Nul: a\0b");
header_remove("x-b");
header_remove("Has: colon");
echo json_encode(headers_list()), "\n";
var_dump(http_response_code(), headers_sent());
