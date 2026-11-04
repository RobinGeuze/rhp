<?php
function reads() {
    var_dump($http_response_header);
}
function assigns_first() {
    $http_response_header = ['x'];
    var_dump($http_response_header);
}
function param($http_response_header) {
    return $http_response_header;
}
function quiet() {
    isset($http_response_header);
    empty($http_response_header);
    unset($http_response_header);
    $http_response_header .= 'a';
    $http_response_header++;
    unknown_function_here($http_response_header);
}
function nested() {
    $http_response_header = 1;
    $f = function () {
        return $http_response_header;
    };
    $g = fn() => $http_response_header;
    return $http_response_header;
}
class C {
    public function m() {
        foreach ([1] as $http_response_header) {}
        return $http_response_header;
    }
    public function n() {
        return strlen($http_response_header) . "$http_response_header";
    }
}
[$http_response_header] = [1];
echo $http_response_header, "\n";
