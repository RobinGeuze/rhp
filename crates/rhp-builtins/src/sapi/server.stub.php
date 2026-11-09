<?php

/** @generate-class-entries */

// The worker-mode functions an embedding server's SAPI has
// (sapi/server.rs), with FrankenPHP's signatures; read by
// `rhp-difftest gen signatures` after php-src's stubs.

function frankenphp_handle_request(callable $callback): bool {}

/** @alias fastcgi_finish_request */
function frankenphp_finish_request(): bool {}
