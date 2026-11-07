<?php require __DIR__.'/../vendor/autoload.php';
$app = require __DIR__.'/../bootstrap/app.php';
$k = $app->make(Illuminate\Contracts\Http\Kernel::class);
$r = $app['router'];
function show($id, $res) { echo "== $id ", $res->getStatusCode(), "\n"; foreach (['Content-Type','Location'] as $h) if ($res->headers->has($h)) echo "$h: ", $res->headers->get($h), "\n"; $b = $res->getContent(); $b = is_string($b) ? preg_replace('/(csrf-token" content="|_token" value=")[^"]+/', '$1<TOK>', $b) : $b; echo is_string($b) ? (strlen($b) > 3000 ? 'len='.strlen($b).' md5='.md5(preg_replace('/\d+(\.\d+)? ?ms/', '', $b))."\n".substr($b,0,600) : $b) : '<stream>', "\n"; }
foreach ([['GET','/'],['GET','/hello/x%20y'],['GET','/api/json?a[]=1&a[]=2&b=%C3%A9'],['GET','/nope'],['POST','/submit'],['GET','/up'],['HEAD','/'],['OPTIONS','/'],['DELETE','/']] as [$m,$u]) { $res = $k->handle($req = Illuminate\Http\Request::create($u, $m)); show("$m $u", $res); $k->terminate($req, $res); }
