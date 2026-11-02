<?php
foreach ([[1 => 1], ['' => 1], ['nope' => 1], ['max_input_vars' => []], ['max_input_vars' => 1.5], ['max_input_vars' => null], ['post_max_size' => '1GB'], ['post_max_size' => 'abc'], ['post_max_size' => '0x'], ['post_max_size' => '12 kb']] as $o) {
    try {
        $r = request_parse_body($o);
        echo count($r[0]), ' ', count($r[1]), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
try { request_parse_body('x'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
