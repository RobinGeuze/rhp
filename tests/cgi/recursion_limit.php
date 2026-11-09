<?php
// Runaway recursion: php's frames fill VM stack pages counted toward
// memory_limit, so it ends in one out-of-memory fatal error.
function r($n) { return r($n + 1); }
r(0);
