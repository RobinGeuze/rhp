<?php
// A filtered stream closes when its last reference goes (assignment, unset,
// array element, property), its filters running then; php://filter with
// no filter spec reads the URL as filter names; a filtered include takes
// one resource id.

class F extends php_user_filter {
    function filter($in, $out, &$consumed, $closing): int {
        while ($b = stream_bucket_make_writeable($in)) {
            $consumed += $b->datalen;
            stream_bucket_append($out, $b);
        }
        if ($closing) echo "[closing]";
        return PSFS_PASS_ON;
    }
    function onClose(): void { echo "[onClose]\n"; }
}
stream_filter_register('my', 'F');
set_error_handler(function ($no, $str) {
    echo "warning: $str\n";
    return true;
});

foreach (['php://memory', 'php://output'] as $url) {
    echo "== $url\n";
    $fp = fopen($url, 'w');
    stream_filter_append($fp, 'my');
    $fp = null;
    echo "after null\n";
    $fp = fopen($url, 'w');
    stream_filter_append($fp, 'my');
    unset($fp);
    echo "after unset\n";
    $a = ['x' => fopen($url, 'w')];
    stream_filter_append($a['x'], 'my');
    unset($a['x']);
    echo "after unset dim\n";
    $a['y'] = fopen($url, 'w');
    stream_filter_append($a['y'], 'my');
    $a['y'] = 1;
    echo "after dim store\n";
    $h = new stdClass;
    $h->fp = fopen($url, 'w');
    stream_filter_append($h->fp, 'my');
    $h->fp = null;
    echo "after prop\n";
    $h->fp = fopen($url, 'w');
    stream_filter_append($h->fp, 'my');
    unset($h->fp);
    echo "after unset prop\n";
    $fp = fopen($url, 'w');
    stream_filter_append($fp, 'my');
    $g = $fp;
    $fp = null;
    echo "still referenced\n";
    $g = 2;
    echo "gone\n";
}

$f = tempnam(sys_get_temp_dir(), 'flt');
file_put_contents($f, "<?php return 5;\n");
var_dump(file_get_contents("php://filter/resource=data:,abc"));
var_dump(fopen('php://memory', 'r'));
var_dump(include "php://filter/read=string.rot13|string.rot13/resource=$f");
var_dump(fopen('php://memory', 'r'));
unlink($f);
