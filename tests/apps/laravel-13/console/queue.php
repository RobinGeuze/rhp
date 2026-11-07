<?php require __DIR__.'/boot.php';
use Illuminate\Support\Facades\{Bus, Event, Cache, RateLimiter, Context, Pipeline, Process, Http, Concurrency};
class J implements Illuminate\Contracts\Queue\ShouldQueue { use Illuminate\Foundation\Bus\Dispatchable, Illuminate\Queue\InteractsWithQueue, Illuminate\Bus\Queueable; public function __construct(public string $m) {} public function handle() { echo "job {$this->m}\n"; } }
class Ev { public function __construct(public int $n) {} }
section('dispatch', function () { J::dispatch('one'); dispatch(new J('two')); Bus::chain([new J('c1'), new J('c2')])->dispatch(); });
$cj = function () {
    echo "closure job\n";
};
section('closure-job', fn() => dispatch($cj));
section('events', function () { Event::listen(Ev::class, fn(Ev $e) => print("got {$e->n}\n")); event(new Ev(3)); Event::listen('a.*', fn($n, $p) => print("$n ".json_encode($p)."\n")); Event::dispatch('a.b', [1,2]); });
section('cache-file', function () { Cache::put('k', ['v'=>[1,2]], 60); Cache::increment('n'); Cache::increment('n', 5); return [Cache::get('k'), Cache::get('n'), Cache::remember('r', 10, fn() => 'computed'), Cache::has('missing'), Cache::lock('l', 5)->get(fn() => 'locked-ok'), Cache::store('array')->many(['a','b'])]; });
section('ratelimiter', function () { foreach (range(1,4) as $i) RateLimiter::hit('key'); return [RateLimiter::tooManyAttempts('key', 3), RateLimiter::remaining('key', 10)]; });
section('pipeline', fn() => Pipeline::send(1)->through([fn($x, $n) => $n($x + 1), fn($x, $n) => $n($x * 10)])->thenReturn());
section('context', function () { Context::add('a', 1); Context::push('b', 'x'); return Context::all(); });
section('process', fn() => [trim(Process::run('echo hi; echo err 1>&2')->output()), Process::run(['printf', '%s', 'a b'])->output(), Process::run('exit 3')->exitCode(), Process::pipe(['echo abc', 'tr a-z A-Z'])->output(), Process::pool(fn($p) => [$p->command('echo p1'), $p->command('echo p2')])->start()->wait()->collect()->map->output()->all()]);
section('http-fake', function () { Http::fake(['example.com/*' => Http::response(['ok'=>true], 201, ['X-A'=>'b'])]); $r = Http::withToken('t')->asJson()->post('https://example.com/api', ['a'=>1]); return [$r->status(), $r->json(), $r->header('X-A')]; });
section('concurrency-sync', fn() => Concurrency::driver('sync')->run([fn() => 1+1, fn() => 'b']));
$c1 = static function () {
    return 2 * 3;
};
$c2 = static function () {
    return strtoupper('x');
};
section('concurrency-process', function () use ($c1, $c2) {
    return Concurrency::run([$c1, $c2]);
});
