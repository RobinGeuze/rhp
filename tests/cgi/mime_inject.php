<?php
// default_mimetype with CR/LF is refused (bug #81518): no header injection.
var_dump(ini_get("default_mimetype"));
ini_set("default_mimetype", "text/html;charset=ISO-8859-1\r\nContent-Length: 31\r\n\r\nLets smuggle a HTTP response.\r\n");
var_dump(ini_get("default_mimetype"), ini_set("default_mimetype", "text/plain\0x"), ini_set("default_mimetype", "text/plain"));
