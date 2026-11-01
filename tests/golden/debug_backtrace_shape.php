<?php
// debug_backtrace() / debug_print_backtrace() share the exception trace shape.
function rel($s) { return str_replace(__DIR__ . '/', '', $s); }
function dump_bt(array $bt) {
    foreach ($bt as $i => $f) {
        $o = isset($f['object']) ? get_class($f['object']) : '-';
        echo "#$i ", implode(",", array_keys($f)), " ", $f['function'], " ",
            basename($f['file'] ?? '-'), ":", $f['line'] ?? '-', " obj=$o",
            " args=", json_encode($f['args'] ?? null), "\n";
    }
}
class Svc {
    public function run($x) { return self::inner($x, 'two'); }
    public static function inner($a, $b) {
        dump_bt(debug_backtrace());
        echo "--\n";
        dump_bt(debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS));
        echo "--\n";
        dump_bt(debug_backtrace(DEBUG_BACKTRACE_PROVIDE_OBJECT, 1));
        echo "--\n";
        ob_start();
        debug_print_backtrace();
        echo rel(ob_get_clean());
        echo "--\n";
        return array_map(function ($v) { dump_bt(debug_backtrace(0)); return $v; }, [1]);
    }
}
(new Svc())->run(5);
function top() { dump_bt(debug_backtrace()); }
top();
dump_bt(debug_backtrace());
echo "end\n";
