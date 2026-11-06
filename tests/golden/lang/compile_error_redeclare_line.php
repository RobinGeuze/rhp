<?php
// A duplicate function is reported at its `function` keyword, after attributes.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval("#[A]\nfunction foo()\n{\n}\n#[B]\nfunction foo() {}");
