<?php
// XMLW_NAME_CHK: every name-taking XMLWriter call validates the name with
// xmlValidateName; the argument number is the procedural one, so the method
// form names whatever parameter sits at that position (if any).
$w = new XMLWriter();
$w->openMemory();
$calls = [
    'startAttribute' => ['1x'], 'writeAttribute' => ['1x', 'v'], 'startAttributeNs' => [null, '1x', null],
    'writeAttributeNs' => ['p', 'a b', 'u', 'v'], 'startElement' => [''], 'startElementNs' => ['p', '-x', 'u'],
    'writeElement' => ['', null], 'writeElementNs' => [null, '1', null], 'startPi' => ['1x'], 'writePi' => ['x y', 'c'],
    'startDtdElement' => ['1'], 'writeDtdElement' => ['1', 'c'], 'startDtdAttlist' => ['1'], 'writeDtdAttlist' => ['1', 'c'],
    'startDtdEntity' => ['1x', false], 'writeDtdEntity' => ['1x', 'c'],
];
foreach ($calls as $m => $a) {
    try { var_dump(call_user_func_array([$w, $m], $a)); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
    $fn = 'xmlwriter_' . strtolower(preg_replace('/([A-Z])/', '_$1', $m));
    try { var_dump(call_user_func_array($fn, [$w, ...$a])); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
// valid names pass through, colons included
var_dump($w->startElement('a:b'), $w->writeAttribute('x:y', '1'), $w->startElementNs('p', 'c', 'urn:p'));
echo $w->outputMemory(), "\n";
