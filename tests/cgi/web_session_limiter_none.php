<?php
header_register_callback(function () {
    // The session id is random and Expires the current time: mask them.
    foreach (headers_list() as $h) {
        if (str_starts_with($h, 'Set-Cookie: PHPSESSID=')) {
            header(preg_replace('/PHPSESSID=\w+/', 'PHPSESSID=ID', $h));
        } elseif (str_starts_with($h, 'Expires: ') && !str_contains($h, '1981')) {
            header('Expires: (now + cache_expire)');
        }
    }
});
ini_set('session.save_path', sys_get_temp_dir());
ini_set('session.use_strict_mode', '1');
session_cache_limiter('none');
session_cache_expire(30);
session_set_cookie_params(['lifetime'=>0,'path'=>'/p','domain'=>'d.example','secure'=>true,'httponly'=>true,'samesite'=>'Strict']);
session_start();
echo "x";
session_write_close();
@unlink(session_save_path() . '/sess_' . session_id());
