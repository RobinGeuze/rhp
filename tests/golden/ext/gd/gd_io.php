<?php
// Writing images to PHP's output, a stream, a path and a user stream, and
// reading them back. Only lossless pixels and sizes are printed: the
// codec builds differ between the reference machines (and Homebrew's libgd
// is built without the GD formats).
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
$dir = sys_get_temp_dir() . '/rhp_gd_io_' . getmypid();
@mkdir($dir);
$im = imagecreatetruecolor(20, 10);
imagefilledrectangle($im, 0, 0, 19, 9, imagecolorallocate($im, 10, 20, 30));
imagesetpixel($im, 3, 4, 0xff00ff);

ob_start();
$ok = imagepng($im);
$png = ob_get_clean();
var_dump($ok);
var_dump(substr($png, 1, 3), getimagesizefromstring($png)[3]);
$back = imagecreatefromstring($png);
var_dump(imagesx($back), imagesy($back), dechex(imagecolorat($back, 3, 4)), dechex(imagecolorat($back, 0, 0)));

$f = fopen('php://memory', 'w+');
var_dump(imagegif($im, $f), ftell($f));
rewind($f);
var_dump(substr(stream_get_contents($f), 0, 6));
fclose($f);

foreach (['png', 'gif', 'bmp', 'wbmp', 'jpeg', 'webp'] as $t) {
    $path = "$dir/out.$t";
    $fn = "image$t";
    var_dump($fn($im, $path), filesize($path) > 0);
    $read = "imagecreatefrom$t";
    $r = $read($path);
    printf("%s %dx%d %s\n", $t, imagesx($r), imagesy($r), imageistruecolor($r) ? 'truecolor' : 'palette');
    unlink($path);
}

class W {
    public $context;
    static $writes = [];
    function stream_open($p, $m, $o, &$op) { return true; }
    function stream_write($d) { self::$writes[] = strlen($d); return strlen($d); }
    function stream_close() {}
}
stream_wrapper_register('wr', 'W');
var_dump(imagepng($im, 'wr://x'));
var_dump(count(W::$writes) > 0, array_sum(W::$writes) === strlen($png));

var_dump(@imagepng($im, "$dir/missing/x.png"));
try { imagepng($im, "a\0b"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { imagepng($im, 1.5); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { imagepng($im, null, 10); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(imagecreatefromstring(str_repeat("x", 20)));
var_dump(imagecreatefrompng(__FILE__));
array_map('unlink', glob("$dir/*"));
rmdir($dir);
