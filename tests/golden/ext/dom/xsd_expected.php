<?php
// XML Schema validation: when libxml2 lists the expected elements.
libxml_use_internal_errors(true);
foreach ([3, 5, 9, 10, 11, 15] as $n) {
  foreach (['choice minOccurs="0" maxOccurs="unbounded"', 'choice', 'sequence', 'choice maxOccurs="unbounded"'] as $grp) {
    $els = '';
    for ($i = 0; $i < $n; $i++) $els .= "<xs:element name=\"e$i\" minOccurs=\"0\"/>";
    $tag = explode(' ', $grp)[0];
    $xsd = "<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><xs:element name=\"r\"><xs:complexType><xs:$grp>$els</xs:$tag></xs:complexType></xs:element></xs:schema>";
    foreach (['<r><zz/></r>', '<r><e1/><zz/></r>'] as $x) {
      $d = new DOMDocument; $d->loadXML($x);
      $d->schemaValidateSource($xsd);
      foreach (libxml_get_errors() as $e) echo "$n $grp $x: ", trim($e->message), "\n";
      libxml_clear_errors();
    }
  }
}
