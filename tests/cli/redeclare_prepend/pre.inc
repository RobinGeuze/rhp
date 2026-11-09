<?php
register_shutdown_function(function () {
    var_dump(function_exists("before_clash"), function_exists("after_clash"));
    if (function_exists("after_clash")) {
        after_clash();
    }
});
