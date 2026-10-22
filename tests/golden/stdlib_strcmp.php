<?php
echo strcmp("a", "b") <=> 0, " ", strcmp("b", "a") <=> 0, " ", strcmp("x", "x"), "\n";
echo strcasecmp("Hello", "hello"), "\n";
echo strncmp("foobar", "foobaz", 5), " ", strncmp("foobar", "foobaz", 6) <=> 0, "\n";
echo strncasecmp("FOO", "foo", 3), "\n";
echo str_ireplace("WORLD", "PHP", "Hello World"), "\n";
echo strpbrk("Hello World", "oW"), "\n";
echo strspn("42 apples", "0123456789"), " ", strcspn("hello", "l"), "\n";
echo str_rot13("Hello World"), "\n";
var_dump(strnatcmp("img10", "img2") > 0);
var_dump(strnatcasecmp("IMG10", "img2") > 0);
echo substr_compare("Hello World", "World", 6), "\n";
