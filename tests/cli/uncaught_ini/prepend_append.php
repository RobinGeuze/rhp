<?php
ini_set('error_prepend_string', '<<');
ini_set('error_append_string', '>>');
throw new Exception('x');
