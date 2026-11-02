<?php
header("Location: /x", true, 307);
header("WWW-Authenticate: Basic");
var_dump(http_response_code());
