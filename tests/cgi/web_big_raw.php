<?php $f = fopen('php://input','r'); $n=0; while(!feof($f)) { $n += strlen(fread($f, 8192)); } echo $n, "\n"; var_dump($_POST);
