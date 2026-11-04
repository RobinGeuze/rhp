<?php
// Included by units.php, then by itself twice: each load has its own
// statics.
static $d = 0;
$d++;
$depth++;
echo "[$d";
if ($depth < 3) {
    include __FILE__;
}
echo "$d]";
