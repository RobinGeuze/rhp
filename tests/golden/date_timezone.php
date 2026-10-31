<?php
var_dump(date_default_timezone_get());
var_dump(date('Y-m-d H:i:s T e P p O Z I c r', 1700000000));
var_dump(gmdate('Y-m-d H:i:s T e P p O Z I c r', 1700000000));
var_dump(date_default_timezone_set('America/New_York'));
var_dump(mktime(0,0,0,3,10,2024), getdate(1710054000)['hours'], date('Y-m-d','86400'));
foreach ([1710054000-1, 1710054000, 1730613600-1, 1730613600, 0, -2000000000, 4102444800, 2000000000] as $t) {
    echo $t, ' ', date('Y-m-d H:i:s T P I e', $t), ' ', gmdate('H:i T', $t), "\n";
}
// gap and ambiguous local times
var_dump(mktime(2,30,0,3,10,2024), mktime(1,30,0,11,3,2024), mktime(3,0,0,3,10,2024), gmmktime(0,0,0,3,10,2024));
var_dump(mktime(0,0,0,1,1,1970), mktime(12,0,0,7,4,1976), mktime(0,0,0,1,1,2100));
var_dump(strtotime('2024-03-10', 0), strtotime('2024-03-10 12:00:00'), strtotime('2024-03-10T12:00:00Z'), strtotime('@86400'), strtotime('+1 day', 1710000000), strtotime('tomorrow', 1710054000), strtotime('today', 1710054000));
print_r(getdate(1730613600));
date_default_timezone_set('Europe/London');
echo date('D, d M Y H:i:s T p', 1700000000), "\n", date('c T', 1720000000), "\n";
date_default_timezone_set('Australia/Sydney');
echo date('c T I', 1700000000), ' ', date('c T I', 1720000000), "\n";
var_dump(mktime(2,30,0,4,7,2024), mktime(2,30,0,10,6,2024));
date_default_timezone_set('Asia/Kolkata');
echo date('c T', 1700000000), "\n";
date_default_timezone_set('America/Sao_Paulo');
echo date('c T Z', 1700000000), "\n";
date_default_timezone_set('UTC');
echo date('c T e', 1700000000), "\n";
ini_set('date.timezone', 'Asia/Tokyo');
echo date('c T e', 1700000000), "\n";
var_dump(date_default_timezone_get());
var_dump(@date_default_timezone_set('Not/AZone'), date_default_timezone_get());
