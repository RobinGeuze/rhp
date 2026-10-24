<?php
// OOP date/time API, fixed inputs only (no "now") so output is stable vs php.
function L($x) { echo $x, "\n"; }

echo "== construct + format ==\n";
$d = new DateTime("2023-11-14 22:13:20");
L($d->format("Y-m-d H:i:s"));
L($d->getTimestamp());
L($d->format(DateTime::ATOM));
L($d->format("c"));
L($d->format("r"));
L($d->format("D, d M Y H:i:s O"));
L($d->format("N S W o w z t L"));
L($d->format("g G h H i s a A"));
L($d->format("U e T O P p Z I B"));
L($d->getTimezone()->getName());

echo "== Y-m-d, @ts, ISO, offset ==\n";
L((new DateTime("2020-02-29"))->format("Y-m-d H:i:s"));
$at = new DateTime("@1700000000");
L($at->format("Y-m-d H:i:s") . " | " . $at->getTimezone()->getName() . " | " . $at->format("e T P"));
$off = new DateTime("2023-11-14 22:13:20", new DateTimeZone("+05:00"));
L($off->format("Y-m-d H:i:s P") . " ts=" . $off->getTimestamp() . " tz=" . $off->getTimezone()->getName());
L($off->format("e T Z O"));
$iso = new DateTime("2023-11-14T22:13:20+02:00");
L($iso->format("Y-m-d H:i:s P") . " ts=" . $iso->getTimestamp() . " tz=" . $iso->getTimezone()->getName());
$isoz = new DateTime("2023-11-14T22:13:20Z");
L($isoz->format("Y-m-d H:i:s P") . " tz=" . $isoz->getTimezone()->getName());
$frac = new DateTime("2023-11-14 22:13:20.123456");
L($frac->format("Y-m-d H:i:s.u v") . " ts=" . $frac->getTimestamp());

echo "== setDate/setTime/setTimestamp/setTimezone ==\n";
$s = new DateTime("2023-01-01 00:00:00");
L($s->setDate(2020, 2, 29)->format("Y-m-d H:i:s"));
L($s->setTime(13, 14, 15)->format("Y-m-d H:i:s"));
L($s->setTime(13, 14, 15, 500000)->format("Y-m-d H:i:s.u"));
L($s->setTimestamp(1700000000)->format("Y-m-d H:i:s"));
L((new DateTime("2023-01-31 10:00:00"))->setDate(2023, 2, 31)->format("Y-m-d"));
$tzc = new DateTime("2023-11-14 22:13:20");
$tzc->setTimezone(new DateTimeZone("+05:00"));
L($tzc->format("Y-m-d H:i:s P") . " ts=" . $tzc->getTimestamp());

echo "== modify ==\n";
L((new DateTime("2023-01-31 10:20:30"))->modify("+1 month")->format("Y-m-d H:i:s"));
L((new DateTime("2023-01-31 10:20:30"))->modify("+1 day")->format("Y-m-d H:i:s"));
L((new DateTime("2023-11-14 22:13:20"))->modify("+2 hours +30 minutes")->format("Y-m-d H:i:s"));
L((new DateTime("2023-11-14 22:13:20"))->modify("-1 week")->format("Y-m-d H:i:s"));
L((new DateTime("2023-01-31"))->modify("+1 year")->format("Y-m-d"));
L((new DateTime("2023-03-15"))->modify("+1 month +1 day")->format("Y-m-d"));
L((new DateTime("2023-11-14 22:13:20"))->modify("+90 minutes")->format("Y-m-d H:i:s"));

echo "== add / sub ==\n";
L((new DateTime("2023-01-31 10:20:30"))->add(new DateInterval("P1M"))->format("Y-m-d H:i:s"));
L((new DateTime("2023-11-14 22:13:20"))->add(new DateInterval("P1Y2M10DT2H30M15S"))->format("Y-m-d H:i:s"));
L((new DateTime("2023-11-14 22:13:20"))->sub(new DateInterval("P1Y2M10DT2H30M15S"))->format("Y-m-d H:i:s"));
L((new DateTime("2023-01-31"))->add(new DateInterval("P1M"))->format("Y-m-d"));
$inv = new DateInterval("P10D");
$inv->invert = 1;
L((new DateTime("2023-06-15"))->add($inv)->format("Y-m-d"));
L((new DateTime("2023-06-15"))->sub($inv)->format("Y-m-d"));
L((new DateTime("2023-06-15"))->add(new DateInterval("P1W"))->format("Y-m-d"));

echo "== diff ==\n";
function diffline($a, $b, $abs = false) {
    $i = (new DateTime($a))->diff(new DateTime($b), $abs);
    return sprintf("y=%d m=%d d=%d h=%d i=%d s=%d days=%d invert=%d", $i->y, $i->m, $i->d, $i->h, $i->i, $i->s, $i->days, $i->invert);
}
L(diffline("2023-11-14 22:13:20", "2025-01-25 00:43:35"));
L(diffline("2025-01-25 00:43:35", "2023-11-14 22:13:20"));
L(diffline("2020-01-31", "2020-03-01"));
L(diffline("2020-03-01", "2020-01-31"));
L(diffline("2020-02-29", "2021-02-28"));
L(diffline("2023-01-01", "2024-01-01"));
L(diffline("2024-02-29 12:00:00", "2023-12-31 23:00:00"));
L(diffline("2023-11-14 22:13:20", "2023-11-14 20:00:00", true));
$id = (new DateTime("2023-01-01"))->diff(new DateTime("2025-04-10"));
L($id->format("%R%a days = %y years, %m months, %d days (%H:%I:%S)"));

echo "== DateInterval ==\n";
$iv = new DateInterval("P1Y2M10DT2H30M15S");
L("$iv->y $iv->m $iv->d $iv->h $iv->i $iv->s $iv->invert");
L($iv->format("%Y-%M-%D %H:%I:%S | %y/%m/%d %h:%i:%s | %R %r | %a | %%"));
$iv2 = new DateInterval("PT90M");
L("$iv2->h $iv2->i $iv2->s");
$iv3 = new DateInterval("P3W");
L("$iv3->d");

echo "== DateTimeImmutable ==\n";
$a = new DateTimeImmutable("2023-11-14 22:13:20");
$b = $a->modify("+1 day");
$c = $a->add(new DateInterval("P1M"));
L($a->format("Y-m-d H:i:s"));
L($b->format("Y-m-d H:i:s"));
L($c->format("Y-m-d"));
L(get_class($b));
L((new DateTimeImmutable("2023-06-01 12:00:00"))->setTime(0, 0, 0)->format("Y-m-d H:i:s"));
var_dump($a instanceof DateTimeInterface);
var_dump($a instanceof DateTimeImmutable);
var_dump($d instanceof DateTimeInterface);
var_dump($d instanceof DateTime);

echo "== DateTimeZone ==\n";
foreach (["UTC", "+05:00", "-0800", "+0530", "GMT"] as $n) {
    $z = new DateTimeZone($n);
    L($n . " => " . $z->getName() . " off=" . (new DateTime("2023-01-01", $z))->getOffset());
}

echo "== errors ==\n";
try { new DateTimeZone("Mars/Phobos"); } catch (\Throwable $e) { L(get_class($e)); }
try { new DateTime("not a date"); } catch (\Throwable $e) { L(get_class($e)); }
try { new DateInterval("garbage"); } catch (\Throwable $e) { L(get_class($e)); }

echo "== constants ==\n";
L(DateTimeInterface::ATOM);
L(DateTimeInterface::COOKIE);
L(DateTimeInterface::RFC2822);
L(DateTime::W3C);
