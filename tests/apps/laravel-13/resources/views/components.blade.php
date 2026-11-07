<x-card tone="info" class="wide" data-x="<&>">
<x-slot:title>T &amp; <b>one</b></x-slot>
Body {{ $who }}
<x-slot:footer>   {{-- only a comment --}}   </x-slot>
</x-card>
<x-card>
<x-slot:title>{{ '<two>' }}</x-slot>
<x-slot:footer class="f">Foot {{ count($items) }}</x-slot>
@foreach ($items as $i => $item)
<span @class(['item', 'first' => $loop->first])>{{ $i }}={{ $item }}</span>
@endforeach
</x-card>
