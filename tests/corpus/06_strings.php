<?php
$s = "The quick brown fox jumps over the lazy dog";
echo strlen($s), "\n";
echo ucwords($s), "\n";
echo strrev($s), "\n";
echo str_word_count($s), "\n";
echo substr_count($s, "o"), "\n";
echo wordwrap($s, 15, "\n", true), "\n";
$words = explode(" ", $s);
echo implode("-", array_map('ucfirst', $words)), "\n";
echo str_pad("x", 10, "-=", STR_PAD_BOTH), "\n";
echo trim("  hi  "), "|", rtrim("hi...", "."), "|", ltrim("xxhi", "x"), "\n";
echo chunk_split("abcdefgh", 3, "/"), "\n";
echo nl2br("a\nb"), "\n";
echo similar_text("World", "word"), "\n";
echo levenshtein("kitten", "sitting"), "\n";
echo soundex("Robert"), " ", metaphone("Thompson"), "\n";
echo ucfirst(strtolower("HELLO WORLD")), "\n";
echo substr_replace("Hello", "XY", 1, 2), "\n";
