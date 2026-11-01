<?php
// SplFixedArray isset, SplObjectStorage (8.5 deprecations, addAll/removeAll/
// seek/serialize), heaps + SplPriorityQueue (php's heap order), SPL iterators.
$a = new SplFixedArray(3); $a[1] = 5; var_dump(isset($a[0]), isset($a[1]), isset($a[5]), $a->offsetExists(1));
$s = new SplObjectStorage; $o1 = new stdClass; $o2 = new stdClass;
$s[$o1] = 'one'; $s->offsetSet($o2); var_dump(count($s), $s[$o1], $s[$o2], isset($s[$o1]));
try { $s[new stdClass]; } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
foreach ($s as $i => $obj) { var_dump($i, $obj === $o1, $s->getInfo()); $s->setInfo("x$i"); }
var_dump($s[$o1], $s[$o2]);
$s->rewind(); $s->seek(1); var_dump($s->key(), $s->current() === $o2);
try { $s->seek(9); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$t = new SplObjectStorage; $t[$o1] = 'T';
var_dump($s->addAll($t), $s[$o1], $s->removeAll($t), count($s), $s->removeAllExcept($t), count($s));
var_dump($s->getHash($o1) === spl_object_hash($o1));
$s->attach($o1, 'att'); var_dump($s->contains($o1)); $s->detach($o1); var_dump(count($s));
$u = new SplObjectStorage; $u[$o1] = [1]; $ser = serialize($u); echo $ser, "\n"; $back = unserialize($ser); var_dump(count($back));
// heaps
$h = new SplMinHeap; foreach ([5, 1, 3, 1, 9, 2] as $v) $h->insert($v); var_dump($h->top(), count($h)); foreach ($h as $k => $v) echo "$k=$v "; echo "\n"; var_dump(count($h));
$h = new SplMaxHeap; foreach ([[2,'a'], [5,'b'], [2,'c'], [5,'d'], [1,'e']] as $v) $h->insert($v); while ($h->valid()) { echo json_encode($h->extract()), ' '; } echo "\n";
try { $h->extract(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
try { $h->top(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
class RevHeap extends SplHeap { protected function compare($a, $b): int { return strlen($a) - strlen($b); } }
$r = new RevHeap; foreach (['aaa', 'b', 'cc', 'dddd', 'ee', 'f'] as $v) $r->insert($v); foreach ($r as $v) echo "$v "; echo "\n";
$pq = new SplPriorityQueue; foreach ([['a', 1], ['b', 3], ['c', 3], ['d', 2], ['e', 3], ['f', 1]] as [$d, $p]) $pq->insert($d, $p);
$pq2 = clone $pq; while (!$pq->isEmpty()) echo $pq->extract(); echo "\n";
$pq2->setExtractFlags(SplPriorityQueue::EXTR_BOTH); var_dump($pq2->top()); $pq2->setExtractFlags(SplPriorityQueue::EXTR_PRIORITY); var_dump($pq2->extract(), count($pq2));
// iterators
$ai = new AppendIterator; $ai->append(new ArrayIterator([1, 2])); $ai->append(new ArrayIterator([])); $ai->append(new ArrayIterator(['a' => 3, 'b' => 4]));
foreach ($ai as $k => $v) { echo "$k=>$v(", $ai->getIteratorIndex(), ") "; } echo "\n";
var_dump(iterator_to_array(new LimitIterator(new ArrayIterator(range(1, 10)), 2, 3)), iterator_to_array(new LimitIterator(new ArrayIterator(range(1, 5)), 3)));
$li = new LimitIterator(new ArrayIterator(range(10, 20)), 2, 5); var_dump($li->seek(4), $li->current(), $li->getPosition());
try { $li->seek(1); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
try { $li->seek(9); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
try { new LimitIterator(new EmptyIterator, -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$g = (function () { yield 1; yield 2; yield 3; })(); $nr = new NoRewindIterator($g); foreach ($nr as $v) { echo $v; break; } foreach ($nr as $v) echo $v; echo "\n";
$n = 0; foreach (new InfiniteIterator(new ArrayIterator([1, 2, 3])) as $v) { echo $v; if (++$n >= 7) break; } echo "\n";
foreach (new EmptyIterator as $v) echo "never"; try { (new EmptyIterator)->current(); } catch (BadMethodCallException $e) { echo $e->getMessage(), "\n"; }
$words = new ArrayIterator(['apple', 'banana', 'cherry', 'k1' => 'avocado', 'date']);
var_dump(iterator_to_array(new RegexIterator($words, '/^a/')));
var_dump(iterator_to_array(new RegexIterator($words, '/^(a)(\w)/', RegexIterator::GET_MATCH)));
var_dump(iterator_to_array(new RegexIterator($words, '/an/', RegexIterator::SPLIT)));
$ri = new RegexIterator($words, '/a/', RegexIterator::REPLACE); $ri->replacement = 'A'; var_dump(iterator_to_array($ri));
var_dump(iterator_to_array(new RegexIterator($words, '/^k/', RegexIterator::MATCH, RegexIterator::USE_KEY)), iterator_to_array(new RegexIterator($words, '/a/', RegexIterator::MATCH, RegexIterator::INVERT_MATCH)));
var_dump(iterator_to_array(new RegexIterator($words, '/a/', RegexIterator::ALL_MATCHES)));
$ci = new CachingIterator(new ArrayIterator(['x', 'y', 'z'])); foreach ($ci as $v) { echo $v, $ci->hasNext() ? ',' : "\n"; } echo (string)$ci, "\n";
$ci = new CachingIterator(new ArrayIterator(['a' => 1, 'b' => 2]), CachingIterator::FULL_CACHE); foreach ($ci as $v); var_dump($ci->getCache(), $ci['a'], count($ci));
try { (new CachingIterator(new EmptyIterator, 0))->__toString(); } catch (BadMethodCallException $e) { echo $e->getMessage(), "\n"; }
try { new CachingIterator(new EmptyIterator, 3); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$mi = new MultipleIterator(); $mi->attachIterator(new ArrayIterator([1, 2, 3])); $mi->attachIterator(new ArrayIterator(['a', 'b']));
foreach ($mi as $k => $v) echo json_encode($k), json_encode($v), ' '; echo "\n";
$mi = new MultipleIterator(MultipleIterator::MIT_NEED_ANY | MultipleIterator::MIT_KEYS_ASSOC); $mi->attachIterator(new ArrayIterator([1, 2, 3]), 'n'); $mi->attachIterator(new ArrayIterator(['a', 'b']), 'l');
foreach ($mi as $v) echo json_encode($v), ' '; echo "\n"; var_dump($mi->countIterators());
try { $mi->attachIterator(new EmptyIterator, 'n'); } catch (InvalidArgumentException $e) { echo $e->getMessage(), "\n"; }
$tree = ['a' => 1, 'b' => ['c' => 2, 'd' => ['e' => 3]], 'f' => 4];
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator($tree)) as $k => $v) echo "$k=$v "; echo "\n";
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::SELF_FIRST) as $k => $v) echo "$k=", is_array($v) ? 'A' : $v, " "; echo "\n";
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::CHILD_FIRST) as $k => $v) echo "$k=", is_array($v) ? 'A' : $v, " "; echo "\n";
$rii = new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::SELF_FIRST); $rii->setMaxDepth(1);
foreach ($rii as $k => $v) echo "$k@", $rii->getDepth(), " "; echo "\n"; var_dump($rii->getMaxDepth());
class Hooks extends RecursiveIteratorIterator { function beginChildren(): void { echo "<"; } function endChildren(): void { echo ">"; } function beginIteration(): void { echo "[begin]"; } function endIteration(): void { echo "[end]"; } }
foreach (new Hooks(new RecursiveArrayIterator($tree)) as $k => $v) echo $k; echo "\n";
foreach (new RecursiveTreeIterator(new RecursiveArrayIterator(['a' => ['b' => 'x', 'c' => ['d' => 'y']], 'e' => 'z'])) as $k => $line) echo "$k: $line\n";
foreach (new ParentIterator(new RecursiveArrayIterator($tree)) as $k => $v) echo $k; echo "\n";
foreach (new RecursiveIteratorIterator(new ParentIterator(new RecursiveArrayIterator($tree)), RecursiveIteratorIterator::SELF_FIRST) as $k => $v) echo $k; echo "\n";
$rr = new RecursiveRegexIterator(new RecursiveArrayIterator(['test1', ['tet3', 'test4', 'test5']]), '/^test/', RecursiveRegexIterator::ALL_MATCHES);
foreach ($rr as $k => $v) { echo $k, json_encode($v), ' '; } echo "\n";
class Subj implements SplSubject { public $obs = []; function attach(SplObserver $o): void { $this->obs[] = $o; } function detach(SplObserver $o): void {} function notify(): void { foreach ($this->obs as $o) $o->update($this); } }
class Obs implements SplObserver { function update(SplSubject $s): void { echo "updated\n"; } }
$sj = new Subj; $sj->attach(new Obs); $sj->notify();
var_dump(new ArrayIterator([1]) instanceof SeekableIterator, new RecursiveArrayIterator([]) instanceof RecursiveIterator);
// seek() back from the end of an iteration leaves php's storage on an invalid position
foreach ($t as $x) {}
$t[$o2] = 2; $t->rewind(); $t->next(); $t->next();
$t->seek(1);
try { $t->current(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
