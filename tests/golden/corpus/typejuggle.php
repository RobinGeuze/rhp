<?php
var_dump(0 == "a");
var_dump("1" == "01");
var_dump("10" == "1e1");
var_dump(100 == "1e2");
var_dump("abc" == 0);
var_dump(null == false);
var_dump([] == false);
var_dump("0" == false);
var_dump(0.0 == "0");
// rhp does not warn "A non-numeric value encountered" yet: silenced.
var_dump(@("1abc" + 5));
var_dump(@("3.14 is pi" + 0));
var_dump(true + true);
var_dump("5 apples" . 3);
var_dump((int)"0x1A");
var_dump((int)"  42  ");
var_dump((float)"1.5e3kg");
var_dump((bool)"0");
var_dump((bool)"0.0");
var_dump((bool)[]);
var_dump(intdiv(-7, 2));
var_dump(-7 % 3);
var_dump(7 % -3);
var_dump(1 <=> 2, 2.5 <=> 2.5, "b" <=> "a");
