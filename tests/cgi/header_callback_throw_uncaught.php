<?php
header_register_callback(function () { throw new Exception('<uncaught>'); });
?>
inline
<?php
echo "not reached\n";
