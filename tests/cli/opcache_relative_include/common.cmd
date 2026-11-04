# a/main.php finds lib.php in its own directory; b/other.php asks for the
# same name, and there is no b/lib.php: the cached a/lib.php must not
# answer it.
script a/main.php
ini display_errors=1
ini error_reporting=-1
ini opcache.enable_cli=1
ini opcache.file_update_protection=0
ini include_path=.
