<?php
// default_charset with CR/LF is refused (bug #81518).
ini_set("default_charset", "UTF-8\r\nX-Evil: 1");
echo "x";
