<?php
echo __LINE__, "\n";
echo "[", __FUNCTION__, "][", __CLASS__, "][", __METHOD__, "][", __NAMESPACE__, "]\n";
function worker() { return __FUNCTION__ . " / " . __METHOD__; }
echo worker(), "\n";
class Service {
    public function handle() {
        $inner = function () { return __FUNCTION__ . "|" . __CLASS__ . "|" . __METHOD__; };
        return __CLASS__ . "::" . __FUNCTION__ . " => " . $inner();
    }
    public static function tag() { return __METHOD__; }
}
echo (new Service)->handle(), "\n";
echo Service::tag(), "\n";
echo __LINE__, "\n";
