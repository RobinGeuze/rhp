<?php
// php-src ext/opcache/tests/match/005.phpt without its opcode dump
// (tests/phpt/ignored.txt): Match expression true

$text = 'Bienvenue chez nous';

$result = match (true) {
    !!preg_match('/Welcome/', $text), !!preg_match('/Hello/', $text) => 'en',
    !!preg_match('/Bienvenue/', $text), !!preg_match('/Bonjour/', $text) => 'fr',
    default => 'other',
};

var_dump($result);

?>
