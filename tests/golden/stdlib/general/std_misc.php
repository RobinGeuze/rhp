<?php
var_dump(ctype_digit("12345"), ctype_digit("12a45"), ctype_alpha("abcDEF"));
var_dump(ctype_space("  \t"), ctype_upper("ABC"));
echo json_encode([1, 2, 3]), "\n";
echo json_encode(["name" => "Bob", "age" => 30, "active" => true]), "\n";
echo json_encode(["nested" => [1, 2], "null" => null, "f" => 3.14]), "\n";
echo json_encode("he said \"hi\"\n"), "\n";
var_dump(is_iterable([1, 2]), is_countable("x"));
