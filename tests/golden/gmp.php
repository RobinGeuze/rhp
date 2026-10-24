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
