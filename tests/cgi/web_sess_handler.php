<?php
class H implements SessionHandlerInterface, SessionUpdateTimestampHandlerInterface {
  public $log = [];
  function open($p, $n): bool { echo "open $n\n"; return true; }
  function close(): bool { echo "close\n"; return true; }
  function read($id): string|false { echo "read $id\n"; return 'a|i:5;'; }
  function write($id, $d): bool { echo "write $id $d\n"; return true; }
  function destroy($id): bool { echo "destroy\n"; return true; }
  function gc($m): int|false { echo "gc\n"; return 0; }
  function validateId($id): bool { echo "validate $id\n"; return true; }
  function updateTimestamp($id, $d): bool { echo "touch $id\n"; return true; }
}
ini_set('session.gc_probability', 0);
session_set_save_handler(new H, true);
session_id('handlerid1234');
session_start(['read_and_close'=>false, 'lazy_write'=>true]);
var_dump($_SESSION);
$_SESSION['a'] = 6;
echo "end\n";
