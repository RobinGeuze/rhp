<form method="post" action="/enum">
<input name="suit" value="{{ old('suit') }}">
<input name="note" value="{{ old('note', 'none') }}">
@error('suit')<p class="error">{{ $message }}</p>@enderror
@if ($errors->any())<p>{{ $errors->count() }} error(s): {{ implode(' | ', $errors->all()) }}</p>@endif
</form>
