<?php require __DIR__.'/boot.php';
use Illuminate\Support\Facades\{Crypt, Hash, URL};
section('crypt', fn() => [Crypt::decryptString(Crypt::encryptString('héllo')), Crypt::decrypt(Crypt::encrypt(['a'=>[1,2]])), strlen(base64_decode(Crypt::encryptString('x')))>0]);
section('bcrypt', fn() => [Hash::check('secret', '$2y$12$xnHmZrE7vU5o8bH5NV3qOuHO4nn8Q1rMJ3.p5JKUP0bD7qSyv8i5u'), Hash::needsRehash('$2y$10$xnHmZrE7vU5o8bH5NV3qOuHO4nn8Q1rMJ3.p5JKUP0bD7qSyv8i5u'), Hash::check('secret', Hash::make('secret')), Hash::info(Hash::make('x'))['algoName']]);
section('argon', fn() => [Hash::driver('argon2id')->check('secret', Hash::driver('argon2id')->make('secret')), Hash::driver('argon')->info(Hash::driver('argon')->make('x'))['algoName']]);
section('signed', function () { app('router')->get('/dl/{id}', fn() => 'x')->name('dl'); app('router')->getRoutes()->refreshNameLookups(); $u = URL::signedRoute('dl', ['id'=>5]); $req = Illuminate\Http\Request::create($u); return [$u, URL::hasValidSignature($req), URL::temporarySignedRoute('dl', \Carbon\Carbon::create(2100,1,1), ['id'=>1])]; });
section('random', fn() => [strlen(Str::random(40)), strlen(Str::password(16)), Str::isUlid((string) Str::ulid()), Str::isUuid((string) Str::uuid7()), strlen(random_bytes(8)), Str::orderedUuid()->getVersion()]);
section('hashes', fn() => [hash('xxh128','a'), hash('sha3-256','a'), hash_hmac('sha256','a','k'), hash('crc32b','a'), md5('a'), base64_encode(hash('sha512', 'a', true))]);
