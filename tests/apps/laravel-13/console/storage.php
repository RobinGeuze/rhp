<?php require __DIR__.'/boot.php';
use Illuminate\Support\Facades\{Storage, File};
section('local', function () { $d = Storage::disk('local'); $d->put('a/b.txt', 'hello'); $d->append('a/b.txt', 'more'); return [$d->get('a/b.txt'), $d->exists('a/b.txt'), $d->size('a/b.txt'), $d->files('a'), $d->allDirectories(), $d->mimeType('a/b.txt'), $d->json('none.json')]; });
section('file', fn() => [File::extension('x.tar.gz'), File::name('/a/b.txt'), File::isFile(base_path('artisan')), count(File::files(config_path())) > 3, File::mimeType(base_path('composer.json')), File::guessExtension(public_path('favicon.ico'))]);
section('upload', function () { $f = Illuminate\Http\UploadedFile::fake()->create('doc.pdf', 10, 'application/pdf'); return [$f->getClientOriginalName(), $f->getSize(), $f->getMimeType(), $f->extension()]; });
