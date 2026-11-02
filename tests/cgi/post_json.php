<?php
var_dump($_POST);
var_dump(json_decode(file_get_contents('php://input'), true));
