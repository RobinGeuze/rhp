<?php
require_once ABSPATH . 'wp-admin/includes/class-wp-debug-data.php'; require_once ABSPATH . 'wp-admin/includes/update.php'; require_once ABSPATH . 'wp-admin/includes/misc.php';
$i = WP_Debug_Data::debug_data(); foreach ($i as $k => $s) { foreach ($s['fields'] as $fk => $f) { echo "$k.$fk=", is_array($f['value']) ? wp_json_encode($f['value']) : $f['value'], "\n"; } }
