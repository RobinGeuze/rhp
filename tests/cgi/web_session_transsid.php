<?php
ini_set('session.save_path', sys_get_temp_dir());
ini_set('session.use_only_cookies', '0');
ini_set('session.use_cookies', '0');
ini_set('session.use_trans_sid', '1');
session_id('transid0123456789');
session_start();
?>
<a href="/page.php?x=1">l</a> <form action="/f.php" method="post"><input name=a></form>
<?php echo SID, "\n";
