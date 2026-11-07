<?php
// A SessionHandler subclass over the files handler: the calls the session
// makes (lazy_write's updateTimestamp, strict mode's validateId,
// regenerate, reset, abort, gc), SID, and the ini checks while a session
// is active.
ob_start();
set_error_handler(function ($no, $msg) { echo "[$no] ", str_replace(__DIR__ . "/", "", $msg), "\n"; return true; });
$dir = sys_get_temp_dir() . '/rhp_golden_session_handlers';
@mkdir($dir);
ini_set('session.save_path', $dir);

class H extends SessionHandler implements SessionUpdateTimestampHandlerInterface {
    public function open(string $path, string $name): bool { echo "open ", basename($path), " $name\n"; return parent::open($path, $name); }
    public function close(): bool { echo "close\n"; return parent::close(); }
    public function read(string $id): string|false { echo "read $id\n"; return parent::read($id); }
    public function write(string $id, string $data): bool { echo "write $id $data\n"; return parent::write($id, $data); }
    public function destroy(string $id): bool { echo "destroy $id\n"; return parent::destroy($id); }
    public function create_sid(): string { echo "create_sid\n"; return 'goldenid' . (++$GLOBALS['n']); }
    public function validateId(string $id): bool { echo "validateId $id\n"; return file_exists(session_save_path() . "/sess_$id"); }
    public function updateTimestamp(string $id, string $data): bool { echo "updateTimestamp $id $data\n"; return true; }
}
$n = 0;
var_dump(session_set_save_handler(new H, false));
var_dump(session_module_name(), session_status());
ini_set('session.use_strict_mode', 1);
session_id('unknown');
var_dump(session_start(), session_id());
$_SESSION['x'] = 1;
var_dump(ini_set('session.name', 'other'), session_name());
var_dump(session_name('again'), session_save_path('/nowhere'), session_id('zzz'));
var_dump(session_cache_limiter('public'), session_cache_expire(5));
var_dump(session_set_cookie_params(10));
var_dump(session_regenerate_id(), session_id());
session_write_close();

echo "-- unchanged data\n";
session_start();
var_dump($_SESSION);
session_write_close();

echo "-- reset, abort\n";
session_start();
$_SESSION['x'] = 2;
var_dump(session_reset(), $_SESSION);
$_SESSION['y'] = 3;
var_dump(session_abort(), session_status());
session_start();
var_dump($_SESSION);
var_dump(session_regenerate_id(true), session_id());
var_dump(session_gc() >= 0);
var_dump(session_destroy(), session_status(), isset($_SESSION));

echo "-- errors\n";
var_dump(session_decode('x|i:1;'), session_regenerate_id(), session_gc(), session_destroy());
var_dump(session_unset(), session_write_close(), session_abort(), session_reset());
var_dump(@session_create_id('bad prefix!'), strlen(session_create_id('ok-')));
try { session_set_cookie_params(['nope' => 1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { session_start(['x' => []]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(session_start(['name' => 'S1', 'read_and_close' => true]), session_status(), session_name());
var_dump(ini_set('session.save_handler', 'nosuch'), ini_set('session.serialize_handler', 'nosuch'));
var_dump(ini_set('session.sid_length', '20'), ini_set('session.name', '123'));

foreach (glob("$dir/sess_*") as $f) unlink($f);
rmdir($dir);
ob_end_flush();
