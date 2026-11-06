<?php
// A callable's deprecation thrown by an error handler: zpp keeps just the
// exception; call_user_func compiled as a user call chains a TypeError.
namespace N;

set_error_handler(function ($no, $str) { throw new \Exception($str); });

function show($e)
{
    echo get_class($e), ": ", $e->getMessage(), "\n";
    $p = $e->getPrevious();
    echo "  previous: ", $p ? get_class($p) . ": " . $p->getMessage() : "none", "\n";
}

class D
{
    static function m() { return 1; }

    static function t()
    {
        try { call_user_func('self::m'); } catch (\Throwable $e) { show($e); }
        try { \call_user_func('self::m'); } catch (\Throwable $e) { show($e); }
        try { \call_user_func_array('self::m', []); } catch (\Throwable $e) { show($e); }
        try { $x = [1, 2]; usort($x, 'self::m'); } catch (\Throwable $e) { show($e); }
        try { \array_filter([1], 'static::m'); } catch (\Throwable $e) { show($e); }
        try { \array_map(['self', 'm'], [1]); } catch (\Throwable $e) { show($e); }
        try { var_dump(is_callable('self::m')); } catch (\Throwable $e) { show($e); }
    }
}

D::t();
