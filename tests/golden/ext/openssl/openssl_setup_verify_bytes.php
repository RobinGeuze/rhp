<?php
// php_openssl_setup_verify names a CA path it cannot stat as given, bytes
// and all (it runs before the certificate is parsed).
ob_start();
var_dump(openssl_x509_checkpurpose("junk", X509_PURPOSE_ANY, ["/nonexistent/ca\xff.pem"]));
echo str_replace(__FILE__, "FILE", ob_get_clean());
