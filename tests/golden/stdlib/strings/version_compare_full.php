<?php
$vs = ['', '1', '1.0', '1.0.0', '1.0.0.0', '1.0-dev', '1.0alpha', '1.0a1', '1.0b', '1.0beta2', '1.0RC1',
    '1.0rc1', '1.0#', '1.0pl1', '1.0p', '1.0.1', '1.10', '1.9', '8.0.0.0-dev', '0.0.0.0-dev', '1.0-foo',
    '1.0abc', '1..0', '1.0.', '#1', '1_0+1', '01.002', 'dev', 'a', '1.0-', '9223372036854775808', '1.0x9'];
foreach ($vs as $a) {
    $row = [];
    foreach ($vs as $b) {
        $row[] = version_compare($a, $b);
    }
    echo str_pad(var_export($a, true), 24), implode(' ', array_map(fn($x) => sprintf('%2d', $x), $row)), "\n";
}
foreach (['<', 'lt', '<=', 'le', '>', 'gt', '>=', 'ge', '==', '=', 'eq', '!=', '<>', 'ne'] as $op) {
    echo $op, ': ', var_export(version_compare('1.0', '1.1', $op), true), ' ', var_export(version_compare('1.1', '1.1', $op), true), "\n";
}
var_dump(version_compare('1', '2', null));
try {
    version_compare('1', '2', 'bogus');
} catch (ValueError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
