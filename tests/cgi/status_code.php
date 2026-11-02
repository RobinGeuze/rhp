<?php
var_dump(http_response_code(418));
var_dump(http_response_code());
header("X-Code: 1", true, 202);
var_dump(http_response_code());
