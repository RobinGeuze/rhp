<?php
// A JPEG with an Exif APP1 built here: what WordPress' image metadata
// reads (Make, Model, Orientation, FNumber, ExposureTime, ISO, dates,
// Copyright, UserComment, GPS), from a file, a stream and with sections.
function ifd(array $entries, int $base, string &$data): string {
    // $entries: [tag, format, count, value bytes]; values over 4 bytes go
    // to $data, which starts at $base.
    $out = pack('v', count($entries));
    $extra = $base + 2 + 12 * count($entries) + 4;
    foreach ($entries as [$tag, $fmt, $count, $bytes]) {
        $out .= pack('vvV', $tag, $fmt, $count);
        if (strlen($bytes) <= 4) {
            $out .= str_pad($bytes, 4, "\0");
        } else {
            $out .= pack('V', $extra + strlen($data));
            $data .= $bytes;
        }
    }
    return $out . pack('V', 0);
}
function build(): string {
    // IFD0 at 8, EXIF IFD at 200, GPS IFD at 500 (offsets from the TIFF header).
    $d0 = '';
    $ifd0 = ifd([
        [0x010F, 2, 6, "Canon\0"],
        [0x0110, 2, 12, "Canon EOS 5\0"],
        [0x0112, 3, 1, pack('v', 6)],
        [0x8298, 2, 13, "Alice\0Bob    \0"],
        [0x8769, 4, 1, pack('V', 200)],
        [0x8825, 4, 1, pack('V', 500)],
    ], 8, $d0);
    $de = '';
    $exif = ifd([
        [0x829A, 5, 1, pack('VV', 1, 250)],
        [0x829D, 5, 1, pack('VV', 28, 10)],
        [0x8827, 3, 1, pack('v', 400)],
        [0x9003, 2, 20, "2024:05:06 07:08:09\0"],
        [0x9004, 2, 20, "2024:05:06 07:08:10\0"],
        [0x920A, 5, 1, pack('VV', 50, 1)],
        [0x9201, 10, 1, pack('VV', 8, 1)],
        [0x9286, 7, 16, "ASCII\0\0\0hi there"],
        [0xA002, 4, 1, pack('V', 640)],
        [0x9999, 9, 2, pack('VV', -5 & 0xFFFFFFFF, 7)],
    ], 200, $de);
    $dg = '';
    $gps = ifd([
        [0x0001, 2, 2, "N\0"],
        [0x0002, 5, 3, pack('VVVVVV', 52, 1, 22, 1, 1234, 100)],
    ], 500, $dg);
    $tiff = 'II' . pack('vV', 0x2A, 8) . $ifd0 . $d0;
    $tiff = str_pad($tiff, 200, "\0") . $exif . $de;
    $tiff = str_pad($tiff, 500, "\0") . $gps . $dg;
    $app1 = "Exif\0\0" . $tiff;
    $sof = "\xFF\xC0" . pack('n', 17) . "\x08" . pack('nn', 3, 4) . "\x03" . str_repeat("\x01\x11\x00", 3);
    return "\xFF\xD8\xFF\xE1" . pack('n', strlen($app1) + 2) . $app1
        . "\xFF\xFE" . pack('n', 7) . "hello" . $sof . "\xFF\xDA" . pack('n', 2) . "\xFF\xD9";
}
set_error_handler(function ($no, $msg) {
    echo "E$no: $msg\n";
    return true;
});
$jpeg = build();
$file = sys_get_temp_dir() . '/rhp_exif_golden.jpg';
file_put_contents($file, $jpeg);

$all = exif_read_data($file);
unset($all['FileDateTime'], $all['FileName']);
var_dump($all);
var_dump(exif_read_data($file, 'GPS', true)['GPS']);
var_dump(exif_read_data($file, 'INTEROP'));

$m = fopen('php://memory', 'w+');
fwrite($m, $jpeg);
fseek($m, 5);
$s = exif_read_data($m, 'FILE,COMPUTED', true);
var_dump($s['FILE'], ftell($m));
var_dump(exif_thumbnail($file), exif_imagetype($file), exif_tagname(0x0112), exif_tagname(-1));
var_dump(exif_read_data(__FILE__), exif_read_data(__DIR__ . '/missing.jpg'));
var_dump(ini_set('exif.encode_unicode', 'nope'), ini_get('exif.encode_unicode'));
unlink($file);
