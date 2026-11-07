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

// Blade components: attribute merging, named slots, hasActualContent().
Route::get('/components', function () {
    return view('components', ['who' => '<Ann>', 'items' => ['x' => 1, 'y' => 'two']]);
});

// Js::from / @js / @json with characters that must be escaped in a script.
Route::get('/js', fn () => view('js'));

// A JSON response with <>&'" (Laravel's default flags leave them as is).
Route::get('/api/chars', fn () => response()->json(['s' => "<a href='x'>&\"</a>", 'u' => "é/\u{2028}"]));

// A sync job and a queued closure (serialized with its `use` variables).
Route::get('/queue', function () {
    \App\Jobs\RecordVisit::dispatch('Bob', ['a' => 1]);
    $n = 21;
    $label = 'closure';
    dispatch(function () use ($n, $label) {
        \Illuminate\Support\Facades\Cache::put('rhp-closure', [$label => $n * 2]);
    });
    return response()->json([
        'job' => \Illuminate\Support\Facades\Cache::pull('rhp-job'),
        'closure' => \Illuminate\Support\Facades\Cache::pull('rhp-closure'),
    ]);
});

Route::get('/period', function () {
    $days = [];
    foreach (\Carbon\CarbonPeriod::create('2026-01-30', '1 day', '2026-02-02') as $d) {
        $days[] = $d->format('D Y-m-d');
    }
    $p = \Carbon\CarbonPeriod::create('2026-01-01', '1 week', 3);
    return response()->json([
        'days' => $days,
        'weeks' => array_map(fn ($d) => $d->toDateString(), $p->toArray()),
        'span' => \Carbon\Carbon::parse('2026-01-01')->diffForHumans(\Carbon\Carbon::parse('2026-03-15'), ['parts' => 2]),
    ]);
});

// Rule::enum: a failing POST redirects back with errors and old input.
Route::get('/enum', fn () => view('enum'));
Route::post('/enum', function (Request $request) {
    $data = $request->validate([
        'suit' => ['required', \Illuminate\Validation\Rule::enum(\App\Enums\Suit::class)],
        'note' => ['nullable', 'string', 'max:5'],
    ]);
    return response()->json(['suit' => \App\Enums\Suit::from($data['suit']), 'note' => $data['note'] ?? null]);
})->withoutMiddleware(\Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class);

Route::post('/upload', function (Request $request) {
    $f = $request->file('doc');
    return response()->json(['name' => $f?->getClientOriginalName(), 'size' => $f?->getSize(), 'valid' => $f?->isValid(), 'client_mime' => $f?->getClientMimeType(), 'ext' => $f?->getClientOriginalExtension(), 'content' => $f ? file_get_contents($f->getRealPath()) : null, 'fields' => $request->except('doc')]);
})->withoutMiddleware(\Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class);
Route::post('/upload-mime', function (Request $request) {
    return response()->json(['mime' => $request->file('doc')->getMimeType()]);
})->withoutMiddleware(\Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class);

Route::get('/number', function () {
    return response()->json([
        'format' => \Illuminate\Support\Number::format(1234567.891, 2),
        'de' => \Illuminate\Support\Number::format(1234.5, locale: 'de'),
        'percent' => \Illuminate\Support\Number::percentage(12.345, 1),
        'currency' => \Illuminate\Support\Number::currency(1234.5, 'EUR'),
        'size' => \Illuminate\Support\Number::fileSize(1536, 1),
        'abbr' => \Illuminate\Support\Number::abbreviate(1234567),
        'spell' => \Illuminate\Support\Number::spell(42),
    ]);
});

Route::get('/storage', function () {
    $disk = \Illuminate\Support\Facades\Storage::disk('local');
    $disk->put('rhp/note.txt', "line 1\nline <2>\n");
    $disk->append('rhp/note.txt', 'appended');
    return response()->json([
        'exists' => $disk->exists('rhp/note.txt'),
        'get' => $disk->get('rhp/note.txt'),
        'size' => $disk->size('rhp/note.txt'),
        'files' => $disk->files('rhp'),
        'missing' => $disk->get('rhp/none.txt'),
    ]);
});

// An uncaught exception: a 500 page, and the debug page in the debug pass.
Route::get('/boom', function () {
    throw new \RuntimeException('boom <b>');
});
