<?php
var_dump(is_numeric("123"));
var_dump(is_numeric("1.5e3"));
var_dump(is_numeric(" 123"));
var_dump(is_numeric("123 "));
var_dump(is_numeric("0x1A"));
var_dump(is_numeric("0b11"));
var_dump(is_numeric(".5"));