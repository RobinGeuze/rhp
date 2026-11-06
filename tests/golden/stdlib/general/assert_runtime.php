<?php
// assert() at run time: zend.assertions, a Throwable description, dynamic
// calls, assert_options() and the callback.
namespace {
    ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
    function side(): bool
    {
        echo "evaluated\n";
        return false;
    }

    var_dump(assert(true));
    try {
        assert(side(), "custom");
    } catch (AssertionError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    try {
        assert(false, new LogicException("mine"));
    } catch (LogicException $e) {
        echo "LogicException: ", $e->getMessage(), " line ", $e->getLine(), "\n";
    }
    try {
        assert(false, null);
    } catch (AssertionError $e) {
        var_dump($e->getMessage());
    }
    try {
        assert(false, []);
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }

    var_dump(ini_set("zend.assertions", "0"));
    var_dump(assert(side()));
    $f = 'assert';
    var_dump($f(false));
    var_dump(ini_set("zend.assertions", "-1"));
    var_dump(ini_get("zend.assertions"));
    ini_set("zend.assertions", "1");

    try {
        $f(false);
    } catch (AssertionError $e) {
        var_dump($e->getMessage());
    }

    var_dump(@assert_options(ASSERT_EXCEPTION, 0));
    var_dump(@assert_options(ASSERT_WARNING));
    var_dump(assert(1 > 2));
    var_dump(@assert_options(ASSERT_CALLBACK, function ($file, $line, $null, $desc = "none") {
        echo "callback: ", basename($file), ":$line ", var_export($null, true), " $desc\n";
    }));
    var_dump(assert(1 > 2));
    var_dump(assert(false, "described"));
    var_dump(@assert_options(ASSERT_ACTIVE, 0));
    var_dump(assert(false));
    var_dump(ini_get("assert.active"));
    @assert_options(ASSERT_ACTIVE, 1);
    @assert_options(ASSERT_WARNING, 0);
    var_dump(assert(false));
    try {
        @assert_options(99);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
    @assert_options(ASSERT_CALLBACK, null);
    var_dump(@assert_options(ASSERT_CALLBACK));
    ini_set("assert.warning", "1");
    var_dump(ini_get("assert.warning"));
    var_dump(assert(0 && 1));
}

namespace Ns {
    try {
        assert(1 + 1 == 3);
    } catch (\AssertionError $e) {
        echo $e->getMessage(), "\n";
    }
}
