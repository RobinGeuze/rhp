<?php
function implicit(I&J $x = null, Foo $f = null, int|string $u = null, ?int $n = null) {
    var_dump($x === null ? null : get_class($x), $f, $u, $n);
}
