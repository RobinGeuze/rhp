<?php echo strlen(file_get_contents('php://input')), " ", count($_POST), " ", strlen($_POST['a'] ?? ''), " ", md5(file_get_contents('php://input')), "\n";
