<?php
$f = $_FILES['f']['tmp_name'];
$g = $_FILES['g']['tmp_name'];
$dest = sys_get_temp_dir() . '/rhp-upload-move-' . getmypid() . '.txt';
var_dump(is_uploaded_file($f), is_uploaded_file(__FILE__), is_uploaded_file($_FILES['f']['name']));
var_dump(move_uploaded_file(__FILE__, $dest), file_exists($dest));
var_dump(move_uploaded_file($f, $dest));
var_dump(file_get_contents($dest), file_exists($f), is_uploaded_file($f));
printf("%o\n", fileperms($dest) & 0777);
var_dump(move_uploaded_file($f, $dest));
unlink($dest);
$bad = '/nonexistent-dir-rhp/x.txt';
ob_start();
$r = move_uploaded_file($g, $bad);
echo str_replace($g, 'TMP', ob_get_clean());
var_dump($r, file_exists($g), is_uploaded_file($g));
try { move_uploaded_file("a\0b", $dest); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { is_uploaded_file("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
