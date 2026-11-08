<script>var d = {{ Js::from(['a' => "</script>&'\"<b>", 'n' => 1, 'f' => 1.5, 'u' => "é\u{2028}"]) }}; var e = @js(['x' => '<>&\'"']); var s = @json(['y' => "<i>'&\""]);</script>
