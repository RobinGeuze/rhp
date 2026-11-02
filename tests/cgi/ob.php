<?php
ob_start();
echo "buffered\n";
header("X-After-Buffered-Output: 1");
var_dump(headers_sent());
$b = ob_get_clean();
http_response_code(202);
echo $b;
