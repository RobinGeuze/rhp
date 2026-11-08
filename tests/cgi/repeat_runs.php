<?php
// php-cgi -T warmup,count: the request runs four times in one process
// (globals fresh each time), then the timing goes to stderr.
echo "run ", ++$GLOBALS["n"], "\n";
