<?php
// GH-19803 (timelib 2022.17, php 8.5.9): a time string made only of
// whitespace is an "Empty string" error, like an empty modify().
foreach (["", " ", "  ", "\t\n ", "\v", "\f", "\r"] as $s) {
    echo json_encode($s), "\n";
    foreach (['DateTime', 'DateTimeImmutable'] as $c) {
        try {
            new $c($s);
            echo "$c: ok\n";
        } catch (DateMalformedStringException $e) {
            echo "$c: ", json_encode($e->getMessage()), "\n";
        }
        try {
            $d = (new $c('2020-01-01 00:00:00'))->modify($s);
            echo "$c::modify: ok\n";
        } catch (DateMalformedStringException $e) {
            echo "$c::modify: ", json_encode($e->getMessage()), "\n";
        }
    }
    var_dump(strtotime($s));
}
