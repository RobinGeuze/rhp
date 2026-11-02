<?php
header_register_callback(function () {
    header("X-From-Callback: yes");
    header_remove("X-Remove-Me");
});
header("X-Remove-Me: 1");
echo "body\n";
var_dump(headers_list());
