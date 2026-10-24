<?php
$base = 1700000000;
$cases = ["2023-11-14", "2023-11-14 08:30:00", "11/14/2023", "14 November 2023",
          "Jan 1 2000", "@1700000000", "@-100", "+1 day", "-2 weeks", "+3 months",
          "+1 year 2 months 10 days", "+1 hour", "1 day ago", "next monday",
          "last friday", "first day of next month", "last day of this month"];
foreach ($cases as $c) echo "$c => ", strtotime($c, $base), "\n";
var_dump(strtotime("not a date", $base), strtotime("13/14/2023", $base));
