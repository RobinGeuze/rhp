# A bad openssl.libctx is an E_ERROR at startup that leaves "custom" and
# exit status 255, and the script still runs.
noscript
ini openssl.libctx=bogus
opt -r
opt echo "start\n"; var_dump(ini_get('openssl.libctx'), error_get_last(), ini_set('openssl.libctx', 'default'));
