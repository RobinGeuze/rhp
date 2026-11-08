<?php
// Static properties read, tested and written through self:: and a name.
class Cfg {
    public static $map = ['a' => 1, 'b' => 2];
    private static $chars = 'abc';
    public static int $n = 0;
    static function run($i) {
        $c = self::$chars[$i % 3];
        return isset(self::$map[$c]) ? self::$map[$c] : strlen(self::$chars);
    }
}
$t = 0;
for ($i = 0; $i < 2000000; $i++) { $t += Cfg::run($i); Cfg::$n = $i & 7; $t += Cfg::$n; }
echo $t, "\n";
