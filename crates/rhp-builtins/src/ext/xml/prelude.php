
final class XMLParser {
    public function __sleep(): array { throw new Exception("Serialization of 'XMLParser' is not allowed"); }
    public function __wakeup(): void { throw new Exception("Unserialization of 'XMLParser' is not allowed"); }
    public static $__ok = false;
    public function __construct() {
        if (!self::$__ok) { throw new Error('Cannot directly construct XMLParser, use xml_parser_create() or xml_parser_create_ns() instead'); }
    }
    public $__ns = false; public $__sep = ':'; public $__fold = true; public $__enc = 'UTF-8';
    public $__skipwhite = 0; public $__toffset = 0; public $__obj = null;
    public $__h = [];
    public $__buf = ''; public $__done = 0; public $__err = 0; public $__errpos = [1, 1, 0];
    public $__final = false; public $__end = [1, 1, 0]; public $__parsing = false;
    // xml_parse_into_struct state
    public $__data = null; public $__info = null; public $__level = 0; public $__ltags = [];
    public $__lastwasopen = 0; public $__ctag = null; public $__curtag = 0;
}

function __xmlp_new($enc, $ns, $sep) {
    XMLParser::$__ok = true;
    $p = new XMLParser();
    XMLParser::$__ok = false;
    $p->__ns = $ns; $p->__sep = $sep;
    if ($enc !== null && $enc !== '') { $p->__enc = strtoupper($enc); }
    return $p;
}

function xml_parser_create(?string $encoding = null): XMLParser {
    if ($encoding !== null && !in_array(strtoupper($encoding), ['', 'UTF-8', 'ISO-8859-1', 'US-ASCII'], true)) {
        throw new ValueError('xml_parser_create(): Argument #1 ($encoding) is not a supported source encoding');
    }
    return __xmlp_new($encoding, false, ':');
}

function xml_parser_create_ns(?string $encoding = null, string $separator = ":"): XMLParser {
    if ($encoding !== null && !in_array(strtoupper($encoding), ['', 'UTF-8', 'ISO-8859-1', 'US-ASCII'], true)) {
        throw new ValueError('xml_parser_create_ns(): Argument #1 ($encoding) is not a supported source encoding');
    }
    return __xmlp_new($encoding, true, $separator === '' ? ':' : $separator[0]);
}

function __xmlp_set(XMLParser $p, $slot, $h, $fn, $argn, $argname, $warn = true) {
    if ($h === null || $h === '') { unset($p->__h[$slot]); return true; }
    if (is_string($h) && !is_callable($h)) {
        if ($p->__obj !== null && method_exists($p->__obj, $h)) {
            if ($warn) { __xmlp_deprecated("$fn(): Passing non-callable strings is deprecated since 8.4"); }
            $p->__h[$slot] = [$p->__obj, $h];
            return true;
        }
        throw new TypeError("$fn(): Argument #$argn (\$$argname) must be a callable or null, function \"$h\" not found or invalid function name");
    }
    $p->__h[$slot] = $h;
    return true;
}

function xml_set_object(XMLParser $parser, object $object): true {
    __xmlp_deprecated('Function xml_set_object() is deprecated since 8.4, provide a proper method callable to xml_set_*_handler() functions', true);
    $parser->__obj = $object;
    return true;
}
function xml_set_element_handler(XMLParser $parser, $start_handler, $end_handler): true {
    __xmlp_set($parser, 's', $start_handler, 'xml_set_element_handler', 2, 'start_handler');
    $warned = is_string($start_handler) && $start_handler !== '' && !is_callable($start_handler);
    __xmlp_set($parser, 'e', $end_handler, 'xml_set_element_handler', 3, 'end_handler', !$warned);
    return true;
}
function xml_set_character_data_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'c', $handler, 'xml_set_character_data_handler', 2, 'handler'); }
function xml_set_processing_instruction_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'pi', $handler, 'xml_set_processing_instruction_handler', 2, 'handler'); }
function xml_set_default_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'd', $handler, 'xml_set_default_handler', 2, 'handler'); }
function xml_set_unparsed_entity_decl_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'u', $handler, 'xml_set_unparsed_entity_decl_handler', 2, 'handler'); }
function xml_set_notation_decl_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'n', $handler, 'xml_set_notation_decl_handler', 2, 'handler'); }
function xml_set_external_entity_ref_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'x', $handler, 'xml_set_external_entity_ref_handler', 2, 'handler'); }
function xml_set_start_namespace_decl_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'ns', $handler, 'xml_set_start_namespace_decl_handler', 2, 'handler'); }
function xml_set_end_namespace_decl_handler(XMLParser $parser, $handler): true { return __xmlp_set($parser, 'nse', $handler, 'xml_set_end_namespace_decl_handler', 2, 'handler'); }

function __xmlp_tag(XMLParser $p, $name) {
    $name = __xmlp_encode($name, $p->__enc);
    return $p->__fold ? strtoupper($name) : $name;
}
function __xmlp_strip(XMLParser $p, $tag) {
    if ($p->__toffset == 0) { return $tag; }
    return $p->__toffset >= strlen($tag) ? '' : substr($tag, $p->__toffset);
}
function __xmlp_info(XMLParser $p, $name) {
    if ($p->__info === null) { return; }
    $p->__info[$name][] = $p->__curtag;
    $p->__curtag++;
}

function __xmlp_dispatch(XMLParser $p, array $ev) {
    switch ($ev[0]) {
    case 's':
        $p->__level++;
        $tag = __xmlp_tag($p, $ev[1]);
        $attrs = [];
        foreach ($ev[2] as [$k, $v]) { $attrs[__xmlp_tag($p, $k)] = __xmlp_encode($v, $p->__enc); }
        if (isset($p->__h['s'])) { ($p->__h['s'])($p, __xmlp_strip($p, $tag), $attrs); }
        if ($p->__data !== null) {
            $st = __xmlp_strip($p, $tag);
            __xmlp_info($p, $st);
            $t = ['tag' => $st, 'type' => 'open', 'level' => $p->__level];
            $p->__ltags[$p->__level - 1] = $tag;
            $p->__lastwasopen = 1;
            if ($attrs) { $t['attributes'] = $attrs; }
            $p->__data[] = $t;
            $p->__ctag = array_key_last($p->__data);
        }
        break;
    case 'e':
        $tag = __xmlp_tag($p, $ev[1]);
        if (isset($p->__h['e'])) { ($p->__h['e'])($p, __xmlp_strip($p, $tag)); }
        if ($p->__data !== null) {
            if ($p->__lastwasopen) {
                $p->__data[$p->__ctag]['type'] = 'complete';
            } else {
                $st = __xmlp_strip($p, $tag);
                __xmlp_info($p, $st);
                $p->__data[] = ['tag' => $st, 'type' => 'close', 'level' => $p->__level];
            }
            $p->__lastwasopen = 0;
        }
        unset($p->__ltags[$p->__level - 1]);
        $p->__level--;
        break;
    case 'c':
        $d = __xmlp_encode($ev[1], $p->__enc);
        if (isset($p->__h['c'])) { ($p->__h['c'])($p, $d); }
        if ($p->__data === null) { break; }
        $doprint = false;
        if ($p->__skipwhite) { $doprint = trim($d, " \t\n") !== ''; }
        if ($p->__lastwasopen) {
            if (isset($p->__data[$p->__ctag]['value']) && is_string($p->__data[$p->__ctag]['value'])) {
                $p->__data[$p->__ctag]['value'] .= $d;
            } elseif ($doprint || !$p->__skipwhite) {
                $p->__data[$p->__ctag]['value'] = $d;
            }
        } else {
            $last = array_key_last($p->__data);
            if ($last !== null && ($p->__data[$last]['type'] ?? null) === 'cdata' && isset($p->__data[$last]['value'])) {
                $p->__data[$last]['value'] .= $d;
                break;
            }
            if ($p->__level > 0 && ($doprint || !$p->__skipwhite)) {
                $st = __xmlp_strip($p, $p->__ltags[$p->__level - 1]);
                __xmlp_info($p, $st);
                $p->__data[] = ['tag' => $st, 'value' => $d, 'type' => 'cdata', 'level' => $p->__level];
            }
        }
        break;
    case 'pi':
        if (isset($p->__h['pi'])) { ($p->__h['pi'])($p, __xmlp_encode($ev[1], $p->__enc), __xmlp_encode($ev[2], $p->__enc)); }
        break;
    case 'd':
        if (isset($p->__h['d'])) { ($p->__h['d'])($p, __xmlp_encode($ev[1], $p->__enc)); }
        break;
    case 'ns':
        if (isset($p->__h['ns'])) { ($p->__h['ns'])($p, $ev[1] === null ? false : $ev[1], $ev[2]); }
        break;
    case 'nse':
        if (isset($p->__h['nse'])) { ($p->__h['nse'])($p, $ev[1] === null ? false : $ev[1]); }
        break;
    }
}

function xml_parse(XMLParser $parser, string $data, bool $is_final = false): int {
    if ($parser->__parsing) { throw new Error('Parser must not be called recursively'); }
    return __xmlp_run($parser, $data, $is_final);
}

function __xmlp_run(XMLParser $parser, string $data, bool $is_final): int {
    if ($parser->__err || $parser->__final) { return 0; }
    $parser->__parsing = true;
    try { return __xmlp_feed($parser, $data, $is_final); } finally { $parser->__parsing = false; }
}

function __xmlp_feed(XMLParser $parser, string $data, bool $is_final): int {
    $parser->__buf .= $data;
    $r = __xmlp_parse($parser->__buf, $parser->__ns, $parser->__sep);
    $ev = $r['ev'];
    $err = $r['err'];
    $len = strlen($parser->__buf);
    // an error at the end of an unfinished chunk only means "more data"
    if (!$is_final && $err !== null && $err[4] >= $len) { $err = null; $hold = true; } else { $hold = !$is_final; }
    $n = count($ev);
    if ($hold && $err === null) {
        while ($n > $parser->__done && $ev[$n - 1][0] === 'c') { $n--; }
        // libxml2's push parser needs 4 bytes before it starts
        if ($len < 4) { $n = $parser->__done; }
    }
    for ($i = $parser->__done; $i < $n; $i++) {
        __xmlp_dispatch($parser, $ev[$i]);
    }
    $parser->__done = $n;
    $parser->__end = $r['end'];
    if ($err !== null) {
        $parser->__err = $err[0];
        $parser->__errpos = [$err[1], $err[2], $err[3]];
        return 0;
    }
    if ($is_final) { $parser->__final = true; }
    return 1;
}

function xml_parse_into_struct(XMLParser $parser, string $data, &$values, &$index = null): int|false {
    if ($parser->__parsing) {
        __xmlp_deprecated('xml_parse_into_struct(): Parser must not be called recursively', false, 2);
        return false;
    }
    $parser->__data = [];
    $parser->__info = func_num_args() > 3 ? [] : null;
    $parser->__level = 0; $parser->__ltags = []; $parser->__lastwasopen = 0; $parser->__curtag = 0;
    $r = __xmlp_run($parser, $data, true);
    $values = $parser->__data;
    if (func_num_args() > 3) { $index = $parser->__info; }
    $parser->__data = null; $parser->__info = null;
    return $r;
}

function xml_get_error_code(XMLParser $parser): int { return $parser->__err; }
function xml_get_current_line_number(XMLParser $parser): int { return $parser->__err ? $parser->__errpos[0] : $parser->__end[0]; }
function xml_get_current_column_number(XMLParser $parser): int { return $parser->__err ? $parser->__errpos[1] : $parser->__end[1]; }
function xml_get_current_byte_index(XMLParser $parser): int { return $parser->__err ? $parser->__errpos[2] : $parser->__end[2]; }
function xml_parser_free(XMLParser $parser): bool {
    __xmlp_deprecated('Function xml_parser_free() is deprecated since 8.5, as it has no effect since PHP 8.0', true);
    return true;
}

function xml_parser_set_option(XMLParser $parser, int $option, $value): true {
    switch ($option) {
        case XML_OPTION_CASE_FOLDING: $parser->__fold = (bool) $value; return true;
        case XML_OPTION_SKIP_TAGSTART:
            if ((int) $value < 0) { throw new ValueError('xml_parser_set_option(): Argument #3 ($value) must be between 0 and ' . PHP_INT_MAX . ' for option XML_OPTION_SKIP_TAGSTART'); }
            $parser->__toffset = (int) $value; return true;
        case XML_OPTION_SKIP_WHITE: $parser->__skipwhite = (bool) $value; return true;
        case XML_OPTION_PARSE_HUGE: return true;
        case XML_OPTION_TARGET_ENCODING:
            $e = strtoupper((string) $value);
            if (!in_array($e, ['UTF-8', 'ISO-8859-1', 'US-ASCII'], true)) {
                throw new ValueError('xml_parser_set_option(): Argument #3 ($value) is not a supported target encoding');
            }
            $parser->__enc = $e; return true;
    }
    throw new ValueError('xml_parser_set_option(): Argument #2 ($option) must be a XML_OPTION_* constant');
}

function xml_parser_get_option(XMLParser $parser, int $option): string|int|bool {
    switch ($option) {
        case XML_OPTION_CASE_FOLDING: return $parser->__fold;
        case XML_OPTION_SKIP_TAGSTART: return $parser->__toffset;
        case XML_OPTION_SKIP_WHITE: return (bool) $parser->__skipwhite;
        case XML_OPTION_PARSE_HUGE: return false;
        case XML_OPTION_TARGET_ENCODING: return $parser->__enc;
    }
    throw new ValueError('xml_parser_get_option(): Argument #2 ($option) must be a XML_OPTION_* constant');
}
