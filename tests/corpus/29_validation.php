<?php
function validateForm(array $data): array {
    $errors = [];
    $rules = [
        'email' => fn($v) => filter_var($v, FILTER_VALIDATE_EMAIL) !== false ?: "invalid email",
        'age' => fn($v) => (is_numeric($v) && $v >= 18) ?: "must be 18+",
        'name' => fn($v) => (strlen(trim($v)) > 0) ?: "name required",
    ];
    foreach ($rules as $field => $rule) {
        $val = $data[$field] ?? null;
        $result = $rule($val);
        if ($result !== true) $errors[$field] = $result;
    }
    return $errors;
}
$cases = [
    ['email' => 'a@b.com', 'age' => 25, 'name' => 'Bob'],
    ['email' => 'bad', 'age' => 15, 'name' => ''],
    ['email' => 'x@y.org', 'age' => 'abc', 'name' => 'X'],
];
foreach ($cases as $i => $c) {
    $errs = validateForm($c);
    echo "case $i: " . (empty($errs) ? "valid" : json_encode($errs)) . "\n";
}
var_dump(filter_var("123", FILTER_VALIDATE_INT));
var_dump(filter_var("1.5e3", FILTER_VALIDATE_FLOAT));
var_dump(filter_var("yes", FILTER_VALIDATE_BOOLEAN));
var_dump(filter_var("http://x.com", FILTER_VALIDATE_URL));
