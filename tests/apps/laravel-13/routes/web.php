<?php

use Illuminate\Http\Request;
use Illuminate\Support\Facades\Route;

// The fixture routes the rhp request harness exercises. Keep the output
// deterministic: no clocks, random values or absolute paths in responses.

Route::get('/', function () {
    return view('welcome');
});

Route::get('/hello/{name}', function (string $name) {
    return view('hello', ['name' => $name, 'items' => ['alpha', 'beta', 'gamma']]);
});

Route::get('/api/json', function (Request $request) {
    return response()->json([
        'ok' => true,
        'n' => 42,
        'list' => [1, 2, 3],
        'nested' => ['a' => 'b'],
        'q' => $request->query(),
    ]);
});

// In the web middleware group, so a POST without a CSRF token answers 419.
Route::post('/submit', function (Request $request) {
    return response()->json(['post' => $request->post()]);
});
