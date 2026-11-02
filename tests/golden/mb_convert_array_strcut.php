<?php
// mb_convert_encoding over arrays (keys + nested values), from-encoding lists,
// mb_strcut, mb_substitute_character, mb_list_encodings, mb_encoding_aliases.
var_dump(mb_convert_encoding(['a', 'k' => 'é', ['x', 'ü' => 'ö'], 'é' => 1, 2.5, null, true], 'ASCII', 'UTF-8'));
var_dump(mb_convert_encoding(["caf\xe9", 'n' => ["\xfc"]], 'UTF-8', 'ISO-8859-1'));
var_dump(mb_convert_encoding("caf\xe9", 'UTF-8', 'ISO-8859-1'), mb_convert_encoding('café', 'UTF-8', 'ASCII,UTF-8'));
var_dump(mb_convert_encoding('café', 'ISO-8859-1', ['UTF-8']), bin2hex(mb_convert_encoding('café', 'ISO-8859-1')));
try { mb_convert_encoding('x', 'UTF-8', 'nope'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mb_convert_encoding('x', 'nope'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

var_dump(mb_strcut('abcdef', 2, 3), mb_strcut('aéb€c', 2, 4), mb_strcut('aéb€c', 1), mb_strcut('aéb€c', -3));
var_dump(mb_strcut('aéb€c', 0, -2), mb_strcut('abc', 5), mb_strcut('aéb', 2, 0), mb_strcut('abc', 1, null, 'ASCII'), mb_strcut('€€', 1, 2));

var_dump(mb_substitute_character(), mb_substitute_character('none'), mb_substitute_character(), mb_substitute_character('LONG'), mb_substitute_character(), mb_substitute_character(0x3F), mb_substitute_character());
try { mb_substitute_character(0x110000); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mb_substitute_character('x'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

$all = mb_list_encodings();
var_dump(count($all), $all[0], in_array('UTF-8', $all, true), in_array('ISO-8859-1', $all, true));
var_dump(mb_encoding_aliases('ASCII'), mb_encoding_aliases('utf8'), mb_encoding_aliases('latin1'), mb_strlen('abc', 'us'));
try { mb_encoding_aliases('nope'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
