<?php
http_response_code(304);
header("Content-Type: text/plain");
header("ETag: \"abc\"");
