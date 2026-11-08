<?php for ($i=0;$i<200;$i++) header("X-H$i: ".str_repeat('v', 50)); header('X-Long: '.str_repeat('L', 9000)); echo "ok";
