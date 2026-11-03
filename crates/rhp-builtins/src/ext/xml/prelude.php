final class XMLParser {
    public function __construct() { throw new Error('Cannot directly construct XMLParser, use xml_parser_create() or xml_parser_create_ns() instead'); }
}
function xml_parser_create(?string $encoding = null): XMLParser { return __xml_fn('xml_parser_create', $encoding); }
function xml_parser_create_ns(?string $encoding = null, string $separator = ":"): XMLParser { return __xml_fn('xml_parser_create_ns', $encoding, $separator); }
function xml_set_object($parser, $object): true { return __xml_fn('xml_set_object', $parser, $object); }
function xml_set_element_handler(XMLParser $parser, $start_handler, $end_handler): true { return __xml_fn('xml_set_element_handler', $parser, $start_handler, $end_handler); }
function xml_set_character_data_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_character_data_handler', $parser, $handler); }
function xml_set_processing_instruction_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_processing_instruction_handler', $parser, $handler); }
function xml_set_default_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_default_handler', $parser, $handler); }
function xml_set_unparsed_entity_decl_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_unparsed_entity_decl_handler', $parser, $handler); }
function xml_set_notation_decl_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_notation_decl_handler', $parser, $handler); }
function xml_set_external_entity_ref_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_external_entity_ref_handler', $parser, $handler); }
function xml_set_start_namespace_decl_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_start_namespace_decl_handler', $parser, $handler); }
function xml_set_end_namespace_decl_handler(XMLParser $parser, $handler): true { return __xml_fn('xml_set_end_namespace_decl_handler', $parser, $handler); }
function xml_parse(XMLParser $parser, string $data, bool $is_final = false): int { return __xml_fn('xml_parse', $parser, $data, $is_final); }
function xml_get_error_code(XMLParser $parser): int { return __xml_fn('xml_get_error_code', $parser); }
function xml_error_string($error_code): ?string { return __xml_fn('xml_error_string', $error_code); }
function xml_get_current_line_number(XMLParser $parser): int { return __xml_fn('xml_get_current_line_number', $parser); }
function xml_get_current_column_number(XMLParser $parser): int { return __xml_fn('xml_get_current_column_number', $parser); }
function xml_get_current_byte_index(XMLParser $parser): int { return __xml_fn('xml_get_current_byte_index', $parser); }
function xml_parser_free($parser): bool { return __xml_fn('xml_parser_free', $parser); }
function xml_parser_set_option(XMLParser $parser, $option, $value): bool { return __xml_fn('xml_parser_set_option', $parser, $option, $value); }
function xml_parser_get_option(XMLParser $parser, $option): string|int|bool { return __xml_fn('xml_parser_get_option', $parser, $option); }
