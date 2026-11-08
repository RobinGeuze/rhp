<?php
// php-cgi writes every echo at once: the client gets "first" while the
// script still waits (for a line the test sends only after "first" came).
echo "first\n";
$in = fopen('php://stdin', 'r');
echo "got ", fgets($in);
