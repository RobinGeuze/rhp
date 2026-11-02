<?php
header("X-Not-Sent: 1");
var_dump(headers_sent(), headers_list());
echo "quiet\n";
