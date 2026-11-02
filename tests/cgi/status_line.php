<?php
header("HTTP/1.1 404 Not Found");
var_dump(http_response_code());
var_dump(http_response_code(410));
echo "gone\n";
