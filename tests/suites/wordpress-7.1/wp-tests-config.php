<?php
/**
 * rhp suite ratchet: wordpress-develop's test configuration on SQLite.
 * suites.py copies it into the clone's root, where the test bootstrap looks.
 * The database is the SQLite Database Integration drop-in
 * (src/wp-content/db.php), as in the fixture app tests/apps/wordpress-7.1.
 */

define( 'ABSPATH', __DIR__ . '/src/' );
define( 'WP_DEFAULT_THEME', 'default' );
define( 'WP_DEBUG', true );

// SQLite, in a file the runner deletes before each part. DELETE journal
// mode: no -wal/-shm files left behind by a process that dies.
define( 'DB_ENGINE', 'sqlite' );
define( 'DB_PATH', __DIR__ . '/var/database/wordpress.sqlite' );
define( 'SQLITE_JOURNAL_MODE', 'DELETE' );
define( 'DB_NAME', 'wordpress_tests' );
define( 'DB_USER', '' );
define( 'DB_PASSWORD', '' );
define( 'DB_HOST', '' );
define( 'DB_CHARSET', 'utf8mb4' );
define( 'DB_COLLATE', '' );

// Test-only keys and salts.
define( 'AUTH_KEY', 'rhp-suite-auth-key' );
define( 'SECURE_AUTH_KEY', 'rhp-suite-secure-auth-key' );
define( 'LOGGED_IN_KEY', 'rhp-suite-logged-in-key' );
define( 'NONCE_KEY', 'rhp-suite-nonce-key' );
define( 'AUTH_SALT', 'rhp-suite-auth-salt' );
define( 'SECURE_AUTH_SALT', 'rhp-suite-secure-auth-salt' );
define( 'LOGGED_IN_SALT', 'rhp-suite-logged-in-salt' );
define( 'NONCE_SALT', 'rhp-suite-nonce-salt' );

$table_prefix = 'wptests_';

define( 'WP_TESTS_DOMAIN', 'example.org' );
define( 'WP_TESTS_EMAIL', 'admin@example.org' );
define( 'WP_TESTS_TITLE', 'Test Blog' );

// The engine under test: the bootstrap runs includes/install.php with it.
// suites.py sets WP_PHP_BINARY to the engine's (shell-quoted) command.
define( 'WP_PHP_BINARY', getenv( 'WP_PHP_BINARY' ) ?: 'php' );

define( 'WPLANG', '' );
