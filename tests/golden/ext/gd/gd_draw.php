<?php
// Drawing, palettes, filters and transforms, checked through pixels.
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
$im = imagecreate(8, 8);
$bg = imagecolorallocate($im, 255, 255, 255);
$red = imagecolorallocatealpha($im, 255, 0, 0, 64);
var_dump($bg, $red, imagecolorstotal($im), imagecolorsforindex($im, $red));
imageline($im, 0, 0, 7, 7, $red);
var_dump(imagecolorat($im, 3, 3), imagecolorat($im, 3, 4));
var_dump(imagecolorat($im, 9, 9));
var_dump(imagecolortransparent($im), imagecolortransparent($im, $bg));
var_dump(imagecolorset($im, $red, 0, 0, 255), imagecolorsforindex($im, $red));
var_dump(imageinterlace($im), imageinterlace($im, true), imagegetclip($im));
try { imagecolorallocate($im, 256, 0, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { imagecolorallocatealpha($im, 0, 0, 0, 128); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

$tc = imagecreatetruecolor(6, 4);
imagefill($tc, 0, 0, 0x336699);
imagefilledpolygon($tc, [0, 0, 5, 0, 0, 3], 0xffffff);
for ($y = 0; $y < 4; $y++) {
    $row = [];
    for ($x = 0; $x < 6; $x++) $row[] = sprintf('%06x', imagecolorat($tc, $x, $y));
    echo implode(' ', $row), "\n";
}
var_dump(imagefilter($tc, IMG_FILTER_NEGATE), dechex(imagecolorat($tc, 5, 3)));
var_dump(imagefilter($tc, IMG_FILTER_GRAYSCALE), dechex(imagecolorat($tc, 5, 3)));
try { imagefilter($tc); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
try { imagefilter($tc, IMG_FILTER_BRIGHTNESS); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
try { imagefilter($tc, 99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { imagefilter($tc, IMG_FILTER_SMOOTH, []); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

$flip = imagecreatetruecolor(3, 1);
imagesetpixel($flip, 0, 0, 0xff0000);
imageflip($flip, IMG_FLIP_HORIZONTAL);
var_dump(dechex(imagecolorat($flip, 2, 0)));
$crop = imagecrop($tc, ['x' => 1, 'y' => 1, 'width' => 2, 'height' => 2]);
var_dump(imagesx($crop), imagesy($crop));
$scaled = imagescale($tc, 12);
var_dump(imagesx($scaled), imagesy($scaled), imagegetinterpolation($tc));
$rot = imagerotate($tc, 90, 0);
var_dump(imagesx($rot), imagesy($rot));
$dst = imagecreatetruecolor(3, 2);
var_dump(imagecopyresampled($dst, $tc, 0, 0, 0, 0, 3, 2, 6, 4));
var_dump(imageresolution($tc), imageresolution($tc, 300), imageresolution($tc));
var_dump(imageaffinematrixget(IMG_AFFINE_SCALE, ['x' => 2, 'y' => 3]));
var_dump(imagetruecolortopalette($tc, false, 4), imageistruecolor($tc), imagecolorstotal($tc) <= 4);
var_dump(imagepalettetotruecolor($im), imageistruecolor($im));
var_dump(imagestring($tc, 1, 0, 0, "Hi", 1), imagettfbbox(10, 0, '/nonexistent.ttf', 'x'));
