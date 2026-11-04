<?php
echo strtr("Hello World", "lo", "LO"), "\n";
echo strtr("Hi all", ["Hi" => "Hello", "all" => "everyone"]), "\n";
echo wordwrap("A very long line that needs wrapping", 10, "\n", true), "\n";
echo vsprintf("%s is %d years", ["Bob", 30]), "\n";
echo sprintf("%'.10d", 42), "\n";
echo sprintf("%-'x10s|", "hi"), "\n";
$parts = sscanf("age:42 name:bob", "age:%d name:%s");
print_r($parts);
echo ucwords("hello-world foo_bar", "-_"), "\n";
echo lcfirst("Hello"), "\n";
echo quotemeta("1+1=2"), "\n";
echo addslashes("O'Reilly \"quote\""), "\n";
echo stripslashes("O\\'Reilly"), "\n";
echo nl2br("line1\nline2"), "\n";
echo str_repeat("=-", 5), "\n";
print_r(str_split("hello", 2));
echo count_chars("hello", 1) ? implode(",", array_keys(count_chars("hello", 1))) : "", "\n";
