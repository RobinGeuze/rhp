<?php
echo implode(",", password_algos()), "\n";
$h = '$2y$10$N9qo8uLOickgx2ZMRZoMye1VdLk3fsv36cQ.Rl4bQ2N0J1x8i9Ym';
var_dump(password_verify("secret", $h), password_verify("wrong", $h));
print_r(password_get_info($h));
var_dump(password_needs_rehash($h, PASSWORD_BCRYPT, ["cost" => 10]));
var_dump(password_needs_rehash($h, PASSWORD_BCRYPT, ["cost" => 12]));
