<?php
// HTTP_AUTHORIZATION → PHP_AUTH_USER / PHP_AUTH_PW (php_handle_auth_data).
foreach (['PHP_AUTH_USER', 'PHP_AUTH_PW', 'PHP_AUTH_DIGEST', 'AUTH_TYPE'] as $k) {
    echo $k, ' => ', var_export($_SERVER[$k] ?? null, true), "\n";
}
