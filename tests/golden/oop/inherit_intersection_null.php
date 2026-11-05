<?php
ini_set('display_errors', '0');
register_shutdown_function(function () {
    echo error_get_last()['message'], "\n";
});
// An implicitly nullable intersection prints as the DNF `(A&B)|null`.
require __DIR__ . '/inherit_intersection_null.inc';
