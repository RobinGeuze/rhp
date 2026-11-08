<?php var_dump(function_exists('fastcgi_finish_request'), function_exists('litespeed_finish_request'), function_exists('apache_response_headers'), function_exists('apache_note'));
echo "a\n"; if (function_exists('fastcgi_finish_request')) var_dump(fastcgi_finish_request()); echo "after finish\n";
