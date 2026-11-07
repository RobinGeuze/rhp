<?php
/**
 * rhp fixture: WordPress's configuration (see tests/apps/README.md).
 *
 * WordPress core is downloaded into wordpress/ by `apps.py setup`; it finds
 * this file one directory above ABSPATH. Every value here is fixed and
 * test-only (none of these is a secret), so php and rhp print the same
 * cookies, nonces and hashes.
 */

// The database: SQLite through the SQLite Database Integration drop-in
// (wordpress/wp-content/db.php), kept outside the docroot.
define( 'DB_PATH', __DIR__ . '/var/database/wordpress.sqlite' );
// The plugin's default journal mode, WAL, leaves -wal/-shm files beside the
// database whenever a process exits without closing it (setup does on
// Linux), and the first request then deletes them. With DELETE a request
// that only reads leaves the database directory exactly as it found it.
define( 'SQLITE_JOURNAL_MODE', 'DELETE' );
define( 'DB_NAME', 'wordpress' );
define( 'DB_USER', '' );
define( 'DB_PASSWORD', '' );
define( 'DB_HOST', '' );
define( 'DB_CHARSET', 'utf8mb4' );
define( 'DB_COLLATE', '' );

// Fixed, test-only keys and salts.
define( 'AUTH_KEY',         'rhp-fixture-auth-key-not-a-secret-0123456789' );
define( 'SECURE_AUTH_KEY',  'rhp-fixture-secure-auth-key-not-a-secret-0123' );
define( 'LOGGED_IN_KEY',    'rhp-fixture-logged-in-key-not-a-secret-012345' );
define( 'NONCE_KEY',        'rhp-fixture-nonce-key-not-a-secret-0123456789' );
define( 'AUTH_SALT',        'rhp-fixture-auth-salt-not-a-secret-012345678' );
define( 'SECURE_AUTH_SALT', 'rhp-fixture-secure-auth-salt-not-a-secret-01' );
define( 'LOGGED_IN_SALT',   'rhp-fixture-logged-in-salt-not-a-secret-0123' );
define( 'NONCE_SALT',       'rhp-fixture-nonce-salt-not-a-secret-01234567' );

$table_prefix = 'wp_';

// The site answers on whatever host it is asked on (http://localhost in the
// request diff, 127.0.0.1:<port> under `apps.py smoke`), so it never
// redirects to its canonical host.
$rhp_host = isset( $_SERVER['HTTP_HOST'] ) ? $_SERVER['HTTP_HOST'] : 'localhost';
define( 'WP_HOME', 'http://' . $rhp_host );
define( 'WP_SITEURL', 'http://' . $rhp_host );

// Production settings; nothing reaches the network or runs in the background.
define( 'WP_DEBUG', false );
define( 'WP_ENVIRONMENT_TYPE', 'production' );
define( 'DISABLE_WP_CRON', true );
define( 'AUTOMATIC_UPDATER_DISABLED', true );
define( 'WP_AUTO_UPDATE_CORE', false );
define( 'WP_HTTP_BLOCK_EXTERNAL', true );
define( 'DISALLOW_FILE_MODS', true );

if ( ! defined( 'ABSPATH' ) ) {
	define( 'ABSPATH', __DIR__ . '/wordpress/' );
}

require_once ABSPATH . 'wp-settings.php';
