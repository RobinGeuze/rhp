<?php
// php_json_parser creates an object once its first member is parsed, so
// nested objects take handles in php's order.
var_dump(json_decode('{"a":{"b":{}},"c":1,"d":[{"e":2},{}]}'));
