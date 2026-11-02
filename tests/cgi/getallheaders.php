<?php
var_dump(getallheaders() === apache_request_headers());
var_dump(getallheaders());
header("X-Resp: 1");
header("Content-Type :  text/x");
var_dump(apache_response_headers());
