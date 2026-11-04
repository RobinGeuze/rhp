<?php
// Callbacks are resolved in the scope that set them, as php's FCC is: a
// private method set from inside its class is called by curl_exec.
class P {
    private function cb($ch, $d) { echo "private cb ", strlen($d), "\n"; return strlen($d); }
    protected static function scb($ch, $d) { echo "protected static cb\n"; return strlen($d); }
    private function hdr($ch, $line) { return strlen($line); }
    function set($ch) { return curl_setopt($ch, CURLOPT_WRITEFUNCTION, [$this, 'cb']); }
    function setStatic($ch) { return curl_setopt($ch, CURLOPT_WRITEFUNCTION, [self::class, 'scb']); }
    function setArray($ch) {
        return curl_setopt_array($ch, [CURLOPT_WRITEFUNCTION => [$this, 'cb'], CURLOPT_HEADERFUNCTION => [$this, 'hdr']]);
    }
}
$f = tempnam(sys_get_temp_dir(), 'rhpcurl');
file_put_contents($f, "hello");
foreach (['set', 'setStatic', 'setArray'] as $m) {
    $ch = curl_init("file://$f");
    var_dump((new P)->$m($ch));
    var_dump(curl_exec($ch), curl_errno($ch));
    $copy = curl_copy_handle($ch);
    var_dump(curl_exec($copy));
}
// Outside the class the private method is not callable.
try {
    curl_setopt(curl_init(), CURLOPT_WRITEFUNCTION, [new P, 'cb']);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
unlink($f);
