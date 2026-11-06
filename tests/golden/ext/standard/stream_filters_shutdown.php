<?php
// A write filter that throws as its stream closes: on fclose, on the last
// reference going. (At the end of the request it is reported as uncaught;
// the message holds the path, so no golden pins it.)
class T extends php_user_filter {
  function filter($in, $out, &$consumed, $closing): int {
    while ($b = stream_bucket_make_writeable($in)) { $consumed += $b->datalen; stream_bucket_append($out, $b); }
    if ($closing) throw new RuntimeException("close-throw " . $this->filtername);
    return PSFS_PASS_ON;
  }
  function onClose(): void { echo "onClose {$this->filtername}\n"; }
}
stream_filter_register('t.*', 'T');
$fp = fopen('php://memory', 'w+');
stream_filter_append($fp, 't.a', STREAM_FILTER_WRITE);
fwrite($fp, "x");
try { fclose($fp); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
function g() { $fp = fopen('php://memory', 'w+'); stream_filter_append($fp, 't.b', STREAM_FILTER_WRITE); fwrite($fp, "y"); }
try { g(); echo "after g\n"; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { $q = fopen('php://memory', 'w+'); stream_filter_append($q, 't.c', STREAM_FILTER_WRITE); $q = null; echo "after null\n"; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
echo "end\n";
