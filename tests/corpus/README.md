# rhp differential-testing corpus

Self-contained, single-file PHP scripts used to stress the VM against php 8.5.8.
No file I/O, no include/require/eval, no superglobals. Each `*.php` is run under
both the real `php` binary and the rhp VM and compared byte-for-byte.

Run: `bash run.sh` (from this dir) — prints PASS/FAIL per script.
