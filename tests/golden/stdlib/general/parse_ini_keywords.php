<?php
var_dump(parse_ini_string("a=On\nb=Off\nc=TRUE\nd=False\ne=Yes\nf=NO\ng=None\nh=Null\ni=oN\nj=yES\nk=nULL\nl=trueish\nm=\"On\"\no=  OFF  \np=NoNe;comment"));
var_dump(parse_ini_string("a=On\nb=Off\nc=TRUE\nd=False\ne=Yes\nf=NO\ng=None\nh=Null\ni=nUlL\nj='TRUE'\nk=True2", false, INI_SCANNER_TYPED));
var_dump(parse_ini_string("a=On\nb=TRUE\nc=Null", false, INI_SCANNER_RAW));
var_dump(parse_ini_string("[s]\nx=YES\ny[]=off\ny[]=On", true));
