@props(['tone' => 'plain'])
<div {{ $attributes->merge(['class' => 'card card-'.$tone]) }}>
<h2>{{ $title }}</h2>
{{ $slot }}
@if (isset($footer) && $footer->hasActualContent())<footer {{ $footer->attributes }}>{{ $footer }}</footer>@else<footer>no footer</footer>@endif
</div>
