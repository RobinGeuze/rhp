<?php
$vals = 0;
$x = 2;
switch ($x) {
    case 1:
        echo "one\n";
        break;
    case 2:
        echo "two\n";
    case 3:
        echo "fallthrough to three\n";
        break;
    default:
        echo "default\n";
}
switch ("hello") {
    case "hi": echo "hi\n"; break;
    case "hello": echo "greeting\n"; break;
}
