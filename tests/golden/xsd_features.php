<?php
// XML Schema constructs used by phpunit.xsd / psalm's config.xsd: xs:all,
// group and attributeGroup references, substitution groups, simple content,
// unions, wildcards and a qualified target namespace.
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$xsd = <<<'XSD'
<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns="urn:cfg" targetNamespace="urn:cfg" elementFormDefault="qualified">
  <xs:element name="cfg" type="cfgType"/>
  <xs:complexType name="cfgType">
    <xs:group ref="cfgGroup"/>
    <xs:attributeGroup ref="cfgAttrs"/>
  </xs:complexType>
  <xs:group name="cfgGroup">
    <xs:all>
      <xs:element name="suites" type="suitesType" minOccurs="0"/>
      <xs:element name="php" type="phpType" minOccurs="0"/>
      <xs:element name="plugins" minOccurs="0">
        <xs:complexType><xs:sequence><xs:any minOccurs="0" maxOccurs="unbounded" processContents="skip"/></xs:sequence></xs:complexType>
      </xs:element>
    </xs:all>
  </xs:group>
  <xs:attributeGroup name="cfgAttrs">
    <xs:attribute name="level" type="levelType" default="1"/>
    <xs:attribute name="columns" type="columnsType"/>
    <xs:attribute name="strict" type="xs:boolean"/>
    <xs:anyAttribute namespace="##other" processContents="skip"/>
  </xs:attributeGroup>
  <xs:simpleType name="levelType"><xs:restriction base="xs:integer"/></xs:simpleType>
  <xs:simpleType name="columnsType">
    <xs:union>
      <xs:simpleType><xs:restriction base="xs:integer"/></xs:simpleType>
      <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="max"/></xs:restriction></xs:simpleType>
    </xs:union>
  </xs:simpleType>
  <xs:element name="facet" abstract="true"/>
  <xs:element name="suite" type="suiteType" substitutionGroup="facet"/>
  <xs:complexType name="suitesType">
    <xs:sequence><xs:element ref="facet" maxOccurs="unbounded"/></xs:sequence>
  </xs:complexType>
  <xs:complexType name="suiteType">
    <xs:sequence>
      <xs:choice minOccurs="0" maxOccurs="unbounded">
        <xs:element name="dir" type="dirType"/>
        <xs:element name="file" type="xs:string"/>
      </xs:choice>
      <xs:element name="exclude" type="xs:string" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="name" type="xs:string" use="required"/>
  </xs:complexType>
  <xs:complexType name="dirType">
    <xs:simpleContent>
      <xs:extension base="xs:string">
        <xs:attribute name="suffix" type="xs:string" default="Test.php"/>
        <xs:attribute name="phpVersion" type="xs:string"/>
      </xs:extension>
    </xs:simpleContent>
  </xs:complexType>
  <xs:complexType name="phpType">
    <xs:choice maxOccurs="unbounded">
      <xs:element name="ini" type="nv" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="env" type="nv" minOccurs="0" maxOccurs="unbounded"/>
    </xs:choice>
  </xs:complexType>
  <xs:complexType name="nv">
    <xs:attribute name="name" type="xs:string" use="required"/>
    <xs:attribute name="value" type="xs:string" use="required"/>
  </xs:complexType>
</xs:schema>
XSD;
$docs = [
    '<cfg xmlns="urn:cfg" level="3" columns="max"><suites><suite name="u"><dir suffix=".php">tests</dir><file>a.php</file><exclude>x</exclude></suite></suites><php><ini name="a" value="b"/><env name="E" value="1"/></php><plugins><anything><goes/></anything></plugins></cfg>',
    '<cfg xmlns="urn:cfg" xmlns:o="urn:other" o:extra="1" level="x" columns="wide" strict="yes"/>',
    '<cfg xmlns="urn:cfg" bogus="1"><unknown/></cfg>',
    '<cfg xmlns="urn:cfg"><php/><suites/></cfg>',
    '<cfg xmlns="urn:cfg"><suites><suite><dir><x/></dir></suite><facet/></suites></cfg>',
    '<cfg xmlns="urn:cfg"><suites><suite name="a"><exclude/><dir/></suite></suites><php><ini/></php></cfg>',
    '<cfg><suites/></cfg>',
    '<cfg xmlns="urn:cfg"><php>text<ini name="a" value="b"/></php></cfg>',
];
libxml_use_internal_errors(true);
foreach ($docs as $x) {
    $d = new DOMDocument;
    $d->loadXML($x);
    $r = $d->schemaValidateSource($xsd);
    echo var_export($r, true), "\n";
    foreach (libxml_get_errors() as $e) echo "  [$e->level/$e->code L$e->line] ", trim($e->message), "\n";
    libxml_clear_errors();
}
// invalid schemas
foreach (['<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="cfg" type="nope"/></xs:schema>', '<notaschema/>'] as $s) {
    var_dump($d->schemaValidateSource($s));
    foreach (libxml_get_errors() as $e) echo "  [$e->level/$e->code] ", trim($e->message), "\n";
    libxml_clear_errors();
}
