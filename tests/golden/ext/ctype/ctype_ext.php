<?php
var_dump(ctype_alnum("abc123"), ctype_alnum("abc 123"));
var_dump(ctype_digit("0123"), ctype_digit("12.3"));
var_dump(ctype_cntrl("\n\t"), ctype_graph("abc!"), ctype_print("a b"));
var_dump(ctype_punct("!?."), ctype_space(" \t\n"), ctype_xdigit("deadBEEF"));
echo (int)ctype_upper("ABC"), (int)ctype_lower("abc"), (int)ctype_alpha(""), "\n";
