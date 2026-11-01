<!doctype html>
<html><head><title>Hello {{ $name }}</title></head>
<body>
<h1>Hello, {{ $name }}!</h1>
<ul>
@foreach ($items as $i => $item)
    <li class="{{ $loop->first ? 'first' : '' }}">{{ $loop->iteration }}/{{ $loop->count }} {{ $i }}: {{ strtoupper($item) }}</li>
@endforeach
</ul>
@if (count($items) > 2)
<p>Many items</p>
@else
<p>Few items</p>
@endif
<p>{!! '<b>raw</b>' !!} {{ '<b>escaped</b>' }}</p>
</body></html>
