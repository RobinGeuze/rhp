<?php
// An exception from the header_register_callback callback surfaces at the
// statement whose output sent the headers.
header_register_callback(function () {
    throw new Exception("from callback");
});
try {
    echo "a\n";
} catch (Exception $e) {
    echo "caught: ", $e->getMessage(), "\n";
}
echo "done\n";
