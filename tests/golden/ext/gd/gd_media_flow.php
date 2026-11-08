<?php
// What a CMS media pipeline does with GD (WordPress's WP_Image_Editor_GD,
// Laravel's UploadedFile::fake()->image()): feature checks, a generated
// upload, then thumbnails resized with alpha kept and written to files.
var_dump(extension_loaded('gd'), function_exists('gd_info'), function_exists('imagerotate'));
$types = imagetypes();
foreach (['IMG_GIF', 'IMG_JPEG', 'IMG_PNG', 'IMG_WEBP', 'IMG_AVIF', 'IMG_BMP'] as $c) {
    printf("%s %s\n", $c, ($types & constant($c)) ? 'yes' : 'no');
}
var_dump(function_exists('imageavif'), function_exists('imagewebp'));

// The fake upload: a truecolor image written to a temporary stream.
$fake = imagecreatetruecolor(40, 30);
$tmp = tmpfile();
imagepng($fake, $tmp);
fseek($tmp, 0);
$upload = stream_get_contents($tmp);
fclose($tmp);
var_dump(getimagesizefromstring($upload)['mime']);

// Load, then resize keeping transparency.
$src = imagecreatefromstring($upload);
imagealphablending($src, false);
imagefilledrectangle($src, 0, 0, 19, 29, imagecolorallocatealpha($src, 255, 0, 0, 127));
$dir = sys_get_temp_dir() . '/rhp_gd_media';
@mkdir($dir);
foreach ([[20, 15], [10, 8], [4, 3]] as [$w, $h]) {
    $dst = imagecreatetruecolor($w, $h);
    imagealphablending($dst, false);
    imagesavealpha($dst, true);
    imagefill($dst, 0, 0, imagecolorallocatealpha($dst, 0, 0, 0, 127));
    imagecopyresampled($dst, $src, 0, 0, 0, 0, $w, $h, 40, 30);
    $file = "$dir/thumb-{$w}x{$h}.png";
    var_dump(imagepng($dst, $file));
    $size = getimagesize($file);
    $back = imagecreatefrompng($file);
    $c = imagecolorsforindex($back, imagecolorat($back, 0, 0));
    printf("%dx%d %s alpha=%d\n", $size[0], $size[1], $size['mime'], $c['alpha']);
    unlink($file);
}
$rot = imagerotate($src, 90, 0);
var_dump(imagesx($rot), imagesy($rot));
var_dump(imageflip($rot, IMG_FLIP_VERTICAL));
rmdir($dir);
