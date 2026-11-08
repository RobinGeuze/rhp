<?php header('Cache-Control: no-cache'); header('Last-Modified: '.gmdate('D, d M Y H:i:s', 0).' GMT'); header('ETag: "x"'); http_response_code(304); 
