<?php echo str_repeat('a', 10); header('X-Late: yes'); echo ob_get_level(), ini_get('output_buffering');
print_r(ob_get_status());
