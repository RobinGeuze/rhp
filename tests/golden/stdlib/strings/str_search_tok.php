<?php
// strstr / strchr / stristr / strrchr
var_dump(strstr("user@example.com", "@"));
var_dump(strstr("user@example.com", "@", true));
var_dump(strstr("abc", "x"));
var_dump(strstr("abc", ""));
var_dump(strstr("abc", "", true));
var_dump(strstr("", ""));
var_dump(strstr("aXbXc", "X"));
var_dump(strchr("path/to/file", "/"));
var_dump(stristr("HayStack", "st"));
var_dump(stristr("HayStack", "ST", true));
var_dump(stristr("HayStack", "zz"));
var_dump(stristr("ÄBC", "äbc"));
var_dump(strrchr("a/b/c", "/"));
var_dump(strrchr("a/b/c", "/x"));
var_dump(strrchr("a/b/c", "/", true));
var_dump(strrchr("abc", "z"));
var_dump(strrchr("a\0b", ""));
var_dump(strstr(12345, 3));
var_dump(strrchr("Composer\\Command\\ShowCommand", "\\"));
try {
    strstr([], "a");
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    stristr("a", new stdClass());
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

// strtok
$tok = strtok("This is\tan example\nstring", " \n\t");
while ($tok !== false) {
    echo "[$tok]\n";
    $tok = strtok(" \n\t");
}
var_dump(strtok("/something", "/"));
var_dump(strtok("/"));
var_dump(strtok("///", "/"));
var_dump(strtok("a,,b,", ","));
var_dump(strtok(","));
var_dump(strtok(","));
var_dump(strtok("", ","));
var_dump(strtok("abc", ""));
var_dump(strtok(""));
var_dump(strtok("a b", " "), strtok(","), strtok(","));
// switching delimiters mid-way
var_dump(strtok("k1=v1;k2=v2", "="), strtok(";"), strtok("="), strtok(";"));
