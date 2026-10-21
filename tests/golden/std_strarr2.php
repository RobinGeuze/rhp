<?php
print_r(str_split("hello", 2));
echo strtr("hello", "el", "ip"), "\n";
echo strtr("hi world", ["hi" => "bye", "world" => "earth"]), "\n";
echo substr_replace("Hello World", "PHP", 6), "\n";
echo levenshtein("kitten", "sitting"), "\n";
print_r(array_diff_key(["a"=>1,"b"=>2,"c"=>3], ["b"=>9]));
print_r(array_replace(["a"=>1,"b"=>2], ["b"=>9,"c"=>3]));
echo str_word_count("the quick brown fox"), "\n";
