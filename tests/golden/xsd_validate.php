<?php
// XML Schema validation: libxml2 error codes and messages per construct.
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$xsd = <<<'XSD'
<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="rootType"/>
  <xs:complexType name="rootType">
    <xs:sequence>
      <xs:element name="a" type="xs:integer"/>
      <xs:element name="b" minOccurs="0" maxOccurs="unbounded">
        <xs:complexType>
          <xs:attribute name="req" type="xs:string" use="required"/>
          <xs:attribute name="bool" type="xs:boolean"/>
          <xs:attribute name="en" type="enumType"/>
          <xs:attribute name="loc">
            <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="p"/><xs:enumeration value="q"/></xs:restriction></xs:simpleType>
          </xs:attribute>
          <xs:attribute name="un" type="unionType"/>
        </xs:complexType>
      </xs:element>
      <xs:choice minOccurs="0">
        <xs:element name="c" type="xs:string"/>
        <xs:element name="d" type="xs:string"/>
      </xs:choice>
      <xs:element name="e" minOccurs="0"><xs:complexType><xs:all><xs:element name="x" minOccurs="0"/><xs:element name="y" minOccurs="0"/><xs:element name="z"/></xs:all></xs:complexType></xs:element>
      <xs:element name="f" minOccurs="0" type="xs:positiveInteger"/>
    </xs:sequence>
    <xs:attribute name="ver" type="xs:decimal"/>
  </xs:complexType>
  <xs:simpleType name="enumType"><xs:restriction base="xs:string"><xs:enumeration value="one"/><xs:enumeration value="two"/></xs:restriction></xs:simpleType>
  <xs:simpleType name="unionType"><xs:union><xs:simpleType><xs:restriction base="xs:integer"/></xs:simpleType><xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="max"/></xs:restriction></xs:simpleType></xs:union></xs:simpleType>
</xs:schema>
XSD;
$docs = [
 '<root><a>1</a></root>',
 '<root ver="x"><a>1</a></root>',
 '<root bogus="1"><a>1</a></root>',
 '<root><a>abc</a></root>',
 '<root><b req="1"/></root>',
 '<root><a>1</a><b/></root>',
 '<root><a>1</a><b req="1" bool="maybe" en="three" loc="r" un="min"/></root>',
 '<root><a>1</a><zz/></root>',
 '<root><a>1</a><c/><d/></root>',
 '<root><a>1</a><e><y/><x/></e></root>',
 '<root><a>1</a><e><z/><z/></e></root>',
 '<root><a>1</a>text</root>',
 '<other/>',
 '<root><a>1</a><b req="x"><q/></b></root>',
 '<root><a>1</a><f>0</f></root>',
 '<root><a> 5 </a><b req="1" bool=" true " un="max"/></root>',
 "<root>\n<a>1</a>\n<b req='1'>text</b></root>",
 '<root></root>',
 '<root><a>1</a><b req="1"/><e/></root>',
];
libxml_use_internal_errors(true);
foreach ($docs as $x) {
  $d = new DOMDocument; $d->loadXML($x);
  $r = $d->schemaValidateSource($xsd);
  echo $x, " => ", var_export($r, true), "\n";
  foreach (libxml_get_errors() as $e) echo "  [$e->level/$e->code L$e->line C$e->column] ", json_encode($e->message), "\n";
  libxml_clear_errors();
}
libxml_use_internal_errors(false);
$d = new DOMDocument; $d->loadXML('<root bogus="1"><a>x</a></root>');
var_dump($d->schemaValidateSource($xsd));
var_dump($d->schemaValidateSource('<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="root" type="nope"/></xs:schema>'));
var_dump($d->schemaValidateSource('<notaschema/>'));
var_dump($d->schemaValidateSource('<root/>'));
