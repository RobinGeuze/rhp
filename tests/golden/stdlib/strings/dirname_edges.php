<?php
// dirname() as zend_dirname: slashes around the last component go with it,
// a path of slashes is "/", no slash is ".", and "" stays "".
foreach (['', '/', '///', 'a', 'a/', 'a//b', '//a', '/a/b/c', 'a/b/', '/a/b//c//', '.', '..', '/..', 'a/./b'] as $p) {
    echo json_encode($p), " => ", json_encode(dirname($p)), " | ", json_encode(dirname($p, 2)),
        " | ", json_encode(pathinfo($p, PATHINFO_DIRNAME)), "\n";
}
// Levels stop once the path no longer shrinks.
var_dump(dirname('/a/b/c/d', 3), dirname('a/b', 5), dirname('/x/y', PHP_INT_MAX));
foreach ([0, -1, PHP_INT_MIN] as $levels) {
    try {
        dirname('/a/b', $levels);
    } catch (ValueError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
var_dump(array_key_exists('dirname', pathinfo('')), pathinfo('/')['dirname'], pathinfo('a//b')['dirname']);
// pathinfo: the array only for PATHINFO_ALL, else the first part asked for.
var_dump(pathinfo('/a/b.c', PATHINFO_DIRNAME | PATHINFO_BASENAME),
    pathinfo('/a/b.c', PATHINFO_EXTENSION | PATHINFO_FILENAME),
    pathinfo('/a/b', PATHINFO_EXTENSION), pathinfo('x', 0), pathinfo('/a/b.c', 31),
    pathinfo('', PATHINFO_DIRNAME), pathinfo('.htaccess'));
