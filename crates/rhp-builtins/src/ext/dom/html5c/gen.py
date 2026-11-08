#!/usr/bin/env python3
# Writes glue.c (see head.c) from php-src's ext/dom sources:
#   python3 gen.py <php-src>/ext/dom
import os,sys,re
here=os.path.dirname(os.path.abspath(__file__))
P=sys.argv[1]
hd=open(P+'/html_document.c').read().split('\n')
hp=open(P+'/html5_parser.c').read().split('\n')
hh=open(P+'/html5_parser.h').read().split('\n')
def sl(lines,a,b): return '\n'.join(lines[a-1:b])
out=[]
out.append(open(os.path.join(here,'head.c')).read())
# parser header types (enum..context struct, without prototypes)
h='\n'.join(hh)
start=h.index('typedef enum {')
end=h.index('void lexbor_libxml2_bridge_parse_context_init')
out.append(h[start:end])
# html5_parser.c body
body=sl(hp,33,476)
reps=[
("\tphp_dom_libxml_ns_mapper *ns_mapper = php_dom_ns_mapper_from_private(private_data);\n","    void *ns_mapper = private_data;\n"),
("xmlNsPtr html_ns = php_dom_libxml_ns_mapper_ensure_html_ns(ns_mapper);","xmlNsPtr html_ns = rhp_dom_mapper_get_ns(ns_mapper, \"\", DOM_XHTML_NS_URI, RHP_TOKEN_HTML);"),
("struct lxml_ns {\n\tconst php_dom_ns_magic_token *token;","struct lxml_ns {\n\tint token;"),
("php_dom_ns_is_svg_magic_token","RHP_TOKEN_SVG"),("php_dom_ns_is_mathml_magic_token","RHP_TOKEN_MATHML"),("php_dom_ns_is_html_magic_token","RHP_TOKEN_HTML"),
("""                    zend_string *uri = zend_string_init(ns.href, ns.href_len, false);
                    current_lxml_ns = php_dom_libxml_ns_mapper_get_ns(ns_mapper, NULL, uri);
                    zend_string_release_ex(uri, false);
                    if (EXPECTED(current_lxml_ns != NULL)) {
                        current_lxml_ns->_private = (void *) ns.token;
                    }""","""                    current_lxml_ns = rhp_dom_mapper_get_ns(ns_mapper, "", ns.href, ns.token);"""),
("""					dom_add_element_ns_hook(private_data, lxml_element);
					php_dom_add_templated_content(private_data, lxml_element, lxml_child_parent);""","""					rhp_dom_template_add(private_data, lxml_element, lxml_child_parent);"""),
("""prefixed_xmlns_ns = php_dom_libxml_ns_mapper_get_ns_raw_strings_nullsafe(ns_mapper, "xmlns", DOM_XMLNS_NS_URI);""","""prefixed_xmlns_ns = rhp_dom_mapper_get_ns(ns_mapper, "xmlns", DOM_XMLNS_NS_URI, 0);"""),
("""lxml_attr->ns = php_dom_libxml_ns_mapper_ensure_prefixless_xmlns_ns(ns_mapper);""","""lxml_attr->ns = rhp_dom_mapper_get_ns(ns_mapper, "", DOM_XMLNS_NS_URI, RHP_TOKEN_XMLNS);"""),
("""lxml_attr->ns->_private = (void *) php_dom_ns_is_xmlns_magic_token;""","""rhp_dom_ns_set_token(lxml_attr->ns, RHP_TOKEN_XMLNS);"""),
("""                        xlink_ns = php_dom_libxml_ns_mapper_get_ns_raw_strings_nullsafe(ns_mapper, "xlink", DOM_XLINK_NS_URI);
                        xlink_ns->_private = (void *) php_dom_ns_is_xlink_magic_token;""","""                        xlink_ns = rhp_dom_mapper_get_ns(ns_mapper, "xlink", DOM_XLINK_NS_URI, RHP_TOKEN_XLINK);"""),
("""                        xml_ns = php_dom_libxml_ns_mapper_get_ns_raw_strings_nullsafe(ns_mapper, "xml", DOM_XML_NS_URI);
                        xml_ns->_private = (void *) php_dom_ns_is_xml_magic_token;""","""                        xml_ns = rhp_dom_mapper_get_ns(ns_mapper, "xml", DOM_XML_NS_URI, RHP_TOKEN_XML);"""),
("php_dom_private_data *private_data","void *private_data"),
]
for a,b in reps:
    if a not in body:
        sys.exit("gen.py: php-src changed near: " + a[:70])
    body=body.replace(a,b)
out.append(body)
# html_document.c parse parts
part=sl(hd,38,84)+'\n'+sl(hd,97,767)
reps2=[("php_libxml_pretend_ctx_error_ex(","rhp_html5_report(")]
for a,b in reps2: part=part.replace(a,b)
out.append(part)
out.append(sl(hd,1232,1326))
# lexbor's selector engine adapted to libxml2 (ext/dom/lexbor/selectors-adapted)
sh=open(P+'/lexbor/selectors-adapted/selectors.h').read()
out.append(sh)
sc=open(P+'/lexbor/selectors-adapted/selectors.c').read()
for inc in ['#include <Zend/zend.h>\n','#include <Zend/zend_operators.h>\n','#include <Zend/zend_API.h>\n','#include <php.h>\n','#include "selectors.h"\n','#include "../../namespace_compat.h"\n','#include "../../domexception.h"\n','#include "../../php_dom.h"\n']:
    if inc not in sc:
        sys.exit('gen.py: php-src changed near: '+inc)
    sc=sc.replace(inc,'')
blank='''			if (!EG(exception)) {
				php_dom_throw_error_with_message(NOT_SUPPORTED_ERR, ":blank selector is not implemented because CSSWG has not yet decided its semantics (https://github.com/w3c/csswg-drafts/issues/1967)", true);
			}'''
if blank not in sc:
    sys.exit('gen.py: php-src changed near the :blank selector')
sc=sc.replace(blank,'''			rhp_css_blank = 1;''')
out.append(sc)
out.append(open(os.path.join(here,'tail.c')).read())
s='\n'.join(out)
s=s.replace('zend_result','int').replace('zend_long','long'); s=re.sub(r'\bSUCCESS\b','0',s); s=re.sub(r'\bFAILURE\b','(-1)',s)
s=re.sub(r'\bZEND_ASSERT\(','RHP_ASSERT(',s)
open(os.path.join(here,'glue.c'),'w').write(s)
