<?php
// The entries of an open_basedir list: an empty one makes ini_set fail,
// a trailing ':' adds none.
$d = getcwd();
var_dump(ini_set('open_basedir', "$d::/"));
var_dump(ini_set('open_basedir', ":$d"));
var_dump(ini_set('open_basedir', "$d:"));
var_dump(ini_get('open_basedir') === "$d:");
var_dump(ini_set('open_basedir', ':'));
var_dump(file_exists(basename(__FILE__)), @file_exists('/'));
