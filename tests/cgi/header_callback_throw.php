<?php
header_register_callback(function () { throw new Exception('from callback'); });
try {
    echo "a\n";
} catch (Exception $e) {
    echo 'caught: ', $e->getMessage(), "\n";
}
?>
inline
<?php
echo "done\n";
