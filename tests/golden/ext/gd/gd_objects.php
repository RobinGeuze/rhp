<?php
// GdImage and GdFont: the object handlers and errors gd.c gives them.
$im = imagecreatetruecolor(4, 3);
var_dump($im, $im instanceof GdImage);
try { new GdImage(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new GdFont(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { clone $im; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { serialize($im); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $im->foo = 1; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$other = imagecreatetruecolor(4, 3);
var_dump($im == $im, $im == $other, $im === $other);
var_dump((new ReflectionClass('GdImage'))->isFinal());
var_dump(get_object_vars($im), (array) $im, json_encode($im));
foreach ([1, "x", null, [], new stdClass] as $bad) {
    try { imagesx($bad); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
var_dump(imagefontwidth(0), imagefontwidth(3), imagefontheight(99));
try { imagefontwidth(new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
// An image stays alive while a variable holds it.
$keep = [imagecreate(2, 2)];
var_dump(imagesx($keep[0]), imageistruecolor($keep[0]));
unset($im, $other, $keep);
echo "done\n";
