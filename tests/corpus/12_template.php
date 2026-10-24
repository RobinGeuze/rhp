<?php
function render(string $tpl, array $vars): string {
    return preg_replace_callback('/\{\{\s*(\w+)\s*\}\}/', function ($m) use ($vars) {
        return $vars[$m[1]] ?? '';
    }, $tpl);
}
$tpl = "Dear {{ name }}, you have {{ count }} new messages from {{ sender }}.";
echo render($tpl, ['name' => 'Alice', 'count' => 5, 'sender' => 'Bob']), "\n";

$items = ['Apple', 'Banana', 'Cherry'];
$html = "<ul>\n";
foreach ($items as $i => $item) {
    $html .= sprintf("  <li class=\"item-%d\">%s</li>\n", $i, htmlspecialchars($item));
}
$html .= "</ul>";
echo $html, "\n";

$rows = [['Alice', 30], ['Bob', 25]];
foreach ($rows as [$name, $age]) {
    echo str_pad($name, 10) . str_pad((string)$age, 5, ' ', STR_PAD_LEFT) . "\n";
}
