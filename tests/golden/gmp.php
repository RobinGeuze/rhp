<?php
echo gmp_strval(gmp_add("123456789012345678901234567890", "987654321")), "\n";
echo gmp_strval(gmp_mul("99999999999999999999", "88888888888888888888")), "\n";
echo gmp_strval(gmp_pow("3", "40")), "\n";
echo gmp_strval(gmp_powm("4", "13", "497")), "\n";
echo gmp_strval(gmp_gcd("1071", "462")), " ", gmp_strval(gmp_lcm("21", "6")), "\n";
echo gmp_strval(gmp_div_q("100", "7")), " ", gmp_strval(gmp_mod("100", "7")), "\n";
echo gmp_strval(gmp_sqrt("1000000000000")), " ", gmp_strval(gmp_fact("25")), "\n";
echo gmp_cmp("5", "7"), gmp_sign("-3"), gmp_popcount("255"), "\n";
echo gmp_strval(gmp_and("60", "13")), " ", gmp_strval(gmp_or("60", "13")), " ", gmp_strval(gmp_xor("60", "13")), "\n";
echo gmp_strval(gmp_add("255", "1"), 16), " ", gmp_prob_prime("97"), "\n";
echo (string)gmp_init("42"), " ", gmp_intval("12345"), "\n";
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
echo gmp_strval("-255", -16), " ", gmp_strval("12345678901234567890", 62), " ", gmp_strval(gmp_init("0o17")), "\n";
echo bin2hex(gmp_export("66051", 2, GMP_MSW_FIRST | GMP_BIG_ENDIAN)), " ", gmp_strval(gmp_import("\x01\x02", 1, GMP_LSW_FIRST)), "\n";
$g = gmp_init(-255);
$g->tag = "x";
echo serialize($g), " ", json_encode($g->__serialize()), "\n";
echo unserialize(serialize($g)), " ", unserialize('C:3:"GMP":15:{s:2:"42";a:0:{}}'), "\n";
try { unserialize('O:3:"GMP":1:{i:0;s:1:"x";}'); } catch (Exception $e) { echo $e->getMessage(), " / ", $e->getPrevious()->getMessage(), "\n"; }
echo gmp_init(7) ** 3, " ", pow(gmp_init(2), 70), " ", gmp_init(1) << 70, " ", -gmp_init(5) % 3, " ", ~gmp_init(5), "\n";
try { gmp_init(5) ** -1; } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { gmp_div_qr(1, 2, 7); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { gmp_add([], 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { gmp_setbit($g, [], []); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
print_r(gmp_init(12)); echo "\n";
var_export(gmp_init(12)); echo "\n";
