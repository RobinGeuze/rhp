<?php
class H extends SessionHandler {
  function read($id): string|false { echo "read\n"; return 'a|i:5;'; }
  function write($id, $d): bool { echo "write\n"; return true; }
  function open($p,$n): bool { echo "open\n"; return true; }
  function close(): bool { echo "close\n"; return true; }
}
ini_set('session.use_strict_mode', 0);
session_set_save_handler(new H, true);
session_id('lazyid');
session_start();
echo "unchanged\n";
session_write_close();
session_start(); session_abort(); session_start(); session_reset(); var_dump(session_status()); session_destroy();
