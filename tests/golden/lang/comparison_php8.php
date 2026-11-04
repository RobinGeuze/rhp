<?php
var_dump(0 == "a");
var_dump("1" == "01");
var_dump("10" == "1e1");
var_dump(100 == "1e2");
var_dump("abc" == 0);
var_dump(null == false);
var_dump([] == false);
var_dump("" == null);
var_dump(0 == "");
var_dump("0" == false);