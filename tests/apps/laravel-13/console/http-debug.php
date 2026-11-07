<?php require __DIR__.'/../vendor/autoload.php';
$app = require __DIR__.'/../bootstrap/app.php';
$k = $app->make(Illuminate\Contracts\Http\Kernel::class);
$app->booted(function ($app) { config(['session.driver'=>'array']); $r = $app['router'];
 $r->get('/boom', function () { throw new RuntimeException('Kaboom <b>'); });
 $r->get('/abort', fn() => abort(403, 'Nope'));
 $r->get('/redir', fn() => redirect('/x?y=1')->with('status', 'ok'));
 $r->post('/val', fn(Illuminate\Http\Request $q) => $q->validate(['email'=>'required|email']));
 $r->get('/dl', fn() => response()->streamDownload(fn() => print('abc'), 'f.txt'));
 $r->get('/cookie', fn() => response('c')->cookie('k', 'v', 10));
 $r->get('/model/{user}', fn(App\Models\User $user) => $user);
});
foreach ([['GET','/boom',[]],['GET','/abort',[]],['GET','/redir',[]],['POST','/val',['email'=>'x']],['POST','/val',[],['HTTP_ACCEPT'=>'application/json']],['GET','/dl',[]],['GET','/cookie',[]],['GET','/boom',[],['HTTP_ACCEPT'=>'application/json']]] as $x) { [$m,$u,$p] = $x; $req = Illuminate\Http\Request::create($u, $m, $p, [], [], $x[3] ?? []); $res = $k->handle($req); echo "== $m $u ", $res->getStatusCode(), "\n"; ob_start(); $res->sendContent(); $b = ob_get_clean(); $b = preg_replace(['/\d+(\.\d+)? ?ms/', '/"time":[^,]+/'], '', $b); echo strlen($b) > 2000 ? 'len='.strlen($b)."\n".substr($b, 0, 400)."\n...\n".substr($b, -300) : $b, "\n"; }
