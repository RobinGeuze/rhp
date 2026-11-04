<?php
$graph = [
    'A' => ['B', 'C'],
    'B' => ['D', 'E'],
    'C' => ['F'],
    'D' => [],
    'E' => ['F'],
    'F' => [],
];
function bfs(array $g, string $start): array {
    $visited = [];
    $queue = [$start];
    $order = [];
    while (!empty($queue)) {
        $node = array_shift($queue);
        if (isset($visited[$node])) continue;
        $visited[$node] = true;
        $order[] = $node;
        foreach ($g[$node] as $n) {
            if (!isset($visited[$n])) $queue[] = $n;
        }
    }
    return $order;
}
function dfs(array $g, string $node, array &$visited = []): array {
    $order = [];
    if (isset($visited[$node])) return $order;
    $visited[$node] = true;
    $order[] = $node;
    foreach ($g[$node] as $n) {
        $order = array_merge($order, dfs($g, $n, $visited));
    }
    return $order;
}
echo "BFS: " . implode(" ", bfs($graph, 'A')) . "\n";
echo "DFS: " . implode(" ", dfs($graph, 'A')) . "\n";
