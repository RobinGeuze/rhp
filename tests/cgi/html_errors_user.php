<?php
// trigger_error messages are shown as given under html_errors, even when
// they look like a builtin's "func(): " message (which php escapes).
trigger_error("f(): <b>x</b> & y", E_USER_WARNING);
trigger_error("Foo::bar(): <i>", E_USER_NOTICE);
user_error("g(): <tag>");
strpos("a", "b", 10);
