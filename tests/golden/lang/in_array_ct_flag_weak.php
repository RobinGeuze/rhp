<?php
// Without strict_types the folded in_array() still reads its flag at
// compile time: no null deprecation, and NAN's warning comes first.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
require __DIR__ . '/in_array_ct_flag_weak.inc';
