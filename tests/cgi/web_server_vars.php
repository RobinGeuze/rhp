<?php $s=$_SERVER; unset($s['REQUEST_TIME'],$s['REQUEST_TIME_FLOAT']); var_dump($s, $_GET, $_REQUEST, $_ENV, getenv('HTTP_HOST'), getallheaders());
