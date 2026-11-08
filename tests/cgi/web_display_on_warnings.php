<?php echo $undef; $a = []; echo $a['k']; trigger_error("user <notice>", E_USER_NOTICE); trigger_error("dep", E_USER_DEPRECATED); echo 1/0;
