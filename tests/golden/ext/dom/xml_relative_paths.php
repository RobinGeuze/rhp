<?php
// libxml-based extensions load and save relative paths from the working
// directory chdir() set.
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$base = realpath(sys_get_temp_dir()) . "/rhp_vcwdx_" . bin2hex(random_bytes(6));
mkdir("$base/sub", 0777, true);
$start = getcwd();
chdir($base);
file_put_contents("sub/doc.xml", '<?xml version="1.0"?><!DOCTYPE r SYSTEM "r.dtd"><r a="1"><c>text</c></r>');
file_put_contents("sub/r.dtd", '<!ELEMENT r (c)><!ATTLIST r a CDATA #REQUIRED><!ELEMENT c (#PCDATA)>');
file_put_contents("sub/ent.xml", '<?xml version="1.0"?><!DOCTYPE r [<!ENTITY e SYSTEM "sub/part.txt">]><r>&e;</r>');
file_put_contents("sub/part.txt", "from part");

$d = new DOMDocument();
var_dump($d->load("sub/doc.xml"), $d->documentElement->getAttribute("a"), basename($d->documentURI));
var_dump($d->validate());
var_dump($d->save("out.xml"), file_get_contents("$base/out.xml"));
var_dump($d->saveHTMLFile("out.html") > 0, file_exists("$base/out.html"));
$e = new DOMDocument();
$e->substituteEntities = true;
var_dump($e->loadXML(file_get_contents("sub/ent.xml")), $e->documentElement->textContent);
$s = new DOMDocument();
$s->loadXML('<r a="1"><c>x</c></r>');
file_put_contents("sub/s.rng", '<element name="r" xmlns="http://relaxng.org/ns/structure/1.0"><attribute name="a"/><element name="c"><text/></element></element>');
var_dump($s->relaxNGValidate("sub/s.rng"));

$x = simplexml_load_file("sub/doc.xml");
var_dump((string) $x->c, $x->asXML("sx.xml"), strlen(file_get_contents("$base/sx.xml")) > 0);
var_dump((string) (new SimpleXMLElement("sx.xml", 0, true))->c);

$r = new XMLReader();
var_dump($r->open("sub/doc.xml"), $r->setRelaxNGSchema("sub/s.rng") || true);
while ($r->read()) if ($r->nodeType === XMLReader::ELEMENT) echo $r->name, " ";
echo "\n";
$r->close();

$w = new XMLWriter();
var_dump($w->openUri("sub/w.xml"), $w->startDocument(), $w->writeElement("w", "v"), $w->endDocument());
unset($w);
var_dump(file_get_contents("$base/sub/w.xml"));
$w = xmlwriter_open_uri("w2.xml");
xmlwriter_write_element($w, "x", "y");
xmlwriter_flush($w);
var_dump(file_get_contents("$base/w2.xml"));

foreach (["sub/doc.xml", "sub/r.dtd", "sub/ent.xml", "sub/part.txt", "sub/s.rng", "sub/w.xml", "w2.xml", "out.xml", "out.html", "sx.xml"] as $f) unlink($f);
chdir($start);
var_dump(rmdir("$base/sub"), rmdir($base));
