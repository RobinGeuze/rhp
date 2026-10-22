<?php
var_dump(filter_var("123", FILTER_VALIDATE_INT));
var_dump(filter_var("abc", FILTER_VALIDATE_INT));
var_dump(filter_var("-45", FILTER_VALIDATE_INT));
var_dump(filter_var("12.5", FILTER_VALIDATE_FLOAT));
var_dump(filter_var("1e3", FILTER_VALIDATE_FLOAT));
var_dump(filter_var("yes", FILTER_VALIDATE_BOOLEAN));
var_dump(filter_var("off", FILTER_VALIDATE_BOOLEAN));
var_dump(filter_var("a@b.com", FILTER_VALIDATE_EMAIL));
var_dump(filter_var("not-an-email", FILTER_VALIDATE_EMAIL));
var_dump(filter_var("5", FILTER_VALIDATE_INT, ["options" => ["min_range" => 1, "max_range" => 10]]));
var_dump(filter_var("50", FILTER_VALIDATE_INT, ["options" => ["min_range" => 1, "max_range" => 10]]));
