<?php set_exception_handler(function($e){ echo "handled ", $e->getMessage(), "\n"; http_response_code(500); }); header('X-Before: 1'); throw new Exception("x");
