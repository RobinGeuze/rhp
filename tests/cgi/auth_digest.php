<?php
// A Digest Authorization header → PHP_AUTH_DIGEST, no user/password.
foreach (['PHP_AUTH_USER', 'PHP_AUTH_PW', 'PHP_AUTH_DIGEST'] as $k) {
    echo $k, ' => ', var_export($_SERVER[$k] ?? null, true), "\n";
}
