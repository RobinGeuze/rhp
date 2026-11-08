<?php
setcookie('a', 'v 1;2', ['expires'=>1000000000, 'path'=>'/x', 'domain'=>'example.com', 'secure'=>true, 'httponly'=>true, 'samesite'=>'Lax']);
setcookie('b', '');
setcookie('c', 'x', 1000000000);
setrawcookie('d', 'raw%20v');
setcookie('e[k]', 'arr');
header('Set-Cookie: manual=1', false);
setcookie('a', 'second');
header('X-Dup: 1'); header('X-Dup: 2', false); header('x-dup: 3', false);
header('X-Rep: 1'); header('X-Rep: 2');
header_remove('X-Rep');
header('X-Keep: k');
var_dump(headers_list());
setcookie('bad=name', 'x');
setcookie('ok', 'v', ['samesite'=>'None', 'partitioned'=>true, 'secure'=>true]);
var_dump(setcookie('ok2', 'v', ['bogus'=>1]));
