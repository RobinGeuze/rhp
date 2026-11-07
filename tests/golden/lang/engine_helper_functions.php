<?php
// rhp's prelude classes call engine helpers named `__…`. php has no such
// functions, so user code can neither see nor reach any of them, by any path.
$helpers = [
    '__closure_bind', '__closure_current', '__closure_from_callable', '__collator', '__date',
    '__directory_handle', '__directory_op', '__dom', '__dom_iter', '__dom_prop', '__exception_trace_string', '__fiber', '__finfo',
    '__gmp_construct', '__gmp_serialize', '__gmp_unserialize', '__hash_digest', '__internal_iterator', '__weakmap', '__weakref',
    '__hash_init_options', '__locale', '__normalizer', '__numfmt', '__object_properties_load', '__pdo', '__pdo_row', '__pdo_sqlite', '__pdo_stmt', '__phar_loadphar', '__phar_mapphar',
    '__phar_running', '__phardata_load', '__phardata_save', '__random_engine',
    '__randomizer', '__reflect', '__reflect_ext',
    '__reflect_get_prop', '__reflect_invoke', '__reflect_new_instance_noctor',
    '__reflect_set_prop', '__session_handler', '__spl_array_unserialize', '__spl_deprecated', '__spl_obj_store', '__spl_realpath', '__spl_user_props',
    '__stdext_deprecated', '__stdext_warning', '__sxe', '__sxe_dim', '__sxe_prop',
    '__throwable_string', '__token_list', '__unserialize_destroy', '__unserialize_init', '__unserialize_value', '__xml_fn', '__xmlwriter',
    '__xmlwriter_fn', '__xmlwriter_to', '__xr', '__xr_prop', '__zip',
    '__zip_prop', '__zpp_arg',
];

function attempt(callable|string $f): string
{
    try {
        $f();
        return 'reached';
    } catch (Throwable $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}

$defined = get_defined_functions();
$defined = array_merge($defined['internal'], $defined['user']);
$leaks = 0;
foreach ($helpers as $n) {
    $undefined = "Error: Call to undefined function $n()";
    $badCallback = "function \"$n\" not found or invalid function name";
    $checks = [
        'function_exists' => [function_exists($n), false],
        'is_callable' => [is_callable($n), false],
        'get_defined_functions' => [in_array($n, $defined, true), false],
        'ReflectionFunction' => [
            attempt(fn() => new ReflectionFunction($n)),
            "ReflectionException: Function $n() does not exist",
        ],
        'dynamic call' => [attempt(fn() => $n()), $undefined],
        'dynamic spread call' => [attempt(fn() => $n(...[1])), $undefined],
        'call_user_func' => [
            attempt(fn() => call_user_func($n)),
            "TypeError: call_user_func(): Argument #1 (\$callback) must be a valid callback, $badCallback",
        ],
        'Closure::fromCallable' => [
            attempt(fn() => Closure::fromCallable($n)),
            "TypeError: Failed to create closure from callable: $badCallback",
        ],
        'direct call' => [attempt(fn() => eval("$n();")), $undefined],
        'qualified call' => [attempt(fn() => eval("\\$n(1, 2);")), $undefined],
        'spread call' => [attempt(fn() => eval("$n(...[1]);")), $undefined],
        'named-argument call' => [attempt(fn() => eval("$n(x: 1);")), $undefined],
        'namespaced call' => [
            attempt(fn() => eval("namespace Ns; $n();")),
            "Error: Call to undefined function Ns\\$n()",
        ],
        'first-class callable' => [attempt(fn() => eval("$n(...);")), $undefined],
        'namespaced first-class callable' => [
            attempt(fn() => eval("namespace Ns; $n(...);")),
            "Error: Call to undefined function Ns\\$n()",
        ],
    ];
    foreach ($checks as $what => [$got, $want]) {
        if ($got !== $want) {
            $leaks++;
            echo "$n: $what: ", var_export($got, true), "\n";
        }
    }
}
echo count($helpers), " helpers, $leaks leaks\n";

// A user function may still use the prefix.
function __mine(int $x) { return $x * 2; }
$f = __mine(...);
echo $f(21), " ", var_export(function_exists('__mine'), true), "\n";
echo (strlen(...))('abcd'), "\n";

// In a namespace, `f(...)` finds `Ns\f` first, even when another file
// declares it, then the global `f`.
eval('namespace Ns2; function __xml_fn() { return "Ns2 user"; }');
$f = eval('namespace Ns2; return __xml_fn(...);');
echo (new ReflectionFunction($f))->getName(), ": ", $f(), "\n";
$f = eval('namespace Ns2; return __MINE(...);');
echo (new ReflectionFunction($f))->getName(), ": ", $f(4), "\n";
echo attempt(fn() => eval('namespace Ns2; __XMLWRITER(...);')), "\n";
