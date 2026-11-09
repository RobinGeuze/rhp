//! The exif extension: a port of php-src's ext/exif/exif.c (PHP-3.01):
//! exif_read_data, exif_thumbnail, exif_tagname and exif_imagetype, with
//! the JPEG, TIFF and HEIF scanners, the IFD walker, the maker note tables
//! and the thumbnail rebuild. Files are read through the stream layer as
//! php reads them (seek, tell, read); the tag tables are generated from
//! exif.c (tags.rs). Character set conversion of comments goes through
//! mbstring, as php's zend_multibyte hooks do.
//!
//! exif.c's pointers into its buffers are (buffer, index) pairs here
//! ([`Ptr`], [`OffsetInfo`]). Where php reads memory it never wrote (a
//! one-byte BYTE value in a rebuilt TIFF thumbnail, a short read past a
//! TIFF directory), rhp reads the value itself or zeros.

mod tags;

use std::rc::Rc;

use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use crate::zend::zpp;
use crate::zend::zpp::deref;
use crate::{Host, Thrown, E_NOTICE, E_WARNING};
use tags::TagTable;

const MAX_IFD_NESTING_LEVEL: i32 = 10;
const MAX_IFD_TAGS: i32 = 1000;
const EXIF_MAX_ERRORS: i32 = 10;

const ERROR_FILEEOF: &str = "Unexpected end of file reached";
const ERROR_CORRUPT: &str = "File structure corrupted";
const ERROR_THUMBEOF: &str = "Thumbnail goes IFD boundary or end of file reached";

const BYTES_PER_FORMAT: [usize; 14] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 1];
const NUM_FORMATS: u32 = 13;

const FMT_BYTE: u16 = 1;
const FMT_STRING: u16 = 2;
const FMT_USHORT: u16 = 3;
const FMT_ULONG: u16 = 4;
const FMT_URATIONAL: u16 = 5;
const FMT_SBYTE: u16 = 6;
const FMT_UNDEFINED: u16 = 7;
const FMT_SSHORT: u16 = 8;
const FMT_SLONG: u16 = 9;
const FMT_SRATIONAL: u16 = 10;
const FMT_SINGLE: u16 = 11;
const FMT_DOUBLE: u16 = 12;
const FMT_IFD: u16 = 13;

const TAG_NEW_SUBFILE_IMAGEWIDTH: u32 = 0x0100;
const TAG_IMAGEHEIGHT: u32 = 0x0101;
const TAG_PHOTOMETRIC_INTERPRETATION: u32 = 0x0106;
const TAG_MAKE: u32 = 0x010F;
const TAG_MODEL: u32 = 0x0110;
const TAG_STRIP_OFFSETS: u32 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u32 = 0x0117;
const TAG_SUB_IFD: u32 = 0x014A;
const TAG_JPEG_INTERCHANGE_FORMAT: u32 = 0x0201;
const TAG_JPEG_INTERCHANGE_FORMAT_LEN: u32 = 0x0202;
const TAG_COPYRIGHT: u32 = 0x8298;
const TAG_EXPOSURETIME: u32 = 0x829A;
const TAG_FNUMBER: u32 = 0x829D;
const TAG_EXIF_IFD_POINTER: u32 = 0x8769;
const TAG_GPS_IFD_POINTER: u32 = 0x8825;
const TAG_SHUTTERSPEED: u32 = 0x9201;
const TAG_APERTURE: u32 = 0x9202;
const TAG_MAX_APERTURE: u32 = 0x9205;
const TAG_SUBJECT_DISTANCE: u32 = 0x9206;
const TAG_MAKER_NOTE: u32 = 0x927C;
const TAG_USERCOMMENT: u32 = 0x9286;
const TAG_XP_TITLE: u32 = 0x9C9B;
const TAG_XP_SUBJECT: u32 = 0x9C9F;
const TAG_COMP_IMAGE_WIDTH: u32 = 0xA002;
const TAG_COMP_IMAGE_HEIGHT: u32 = 0xA003;
const TAG_INTEROP_IFD_POINTER: u32 = 0xA005;
const TAG_FOCALPLANE_X_RES: u32 = 0xA20E;
const TAG_FOCALPLANE_RESOLUTION_UNIT: u32 = 0xA210;
/// TAG_NONE (-1) and TAG_COMPUTED_VALUE (-2) as php stores them (WORD).
const TAG_NONE: u16 = 0xFFFF;
const TAG_COMPUTED_VALUE: u16 = 0xFFFE;

const SECTION_FILE: usize = 0;
const SECTION_COMPUTED: usize = 1;
const SECTION_ANY_TAG: usize = 2;
const SECTION_IFD0: usize = 3;
const SECTION_THUMBNAIL: usize = 4;
const SECTION_COMMENT: usize = 5;
const SECTION_APP0: usize = 6;
const SECTION_EXIF: usize = 7;
const SECTION_FPIX: usize = 8;
const SECTION_GPS: usize = 9;
const SECTION_INTEROP: usize = 10;
const SECTION_APP12: usize = 11;
const SECTION_WINXP: usize = 12;
const SECTION_MAKERNOTE: usize = 13;
const SECTION_COUNT: usize = 14;

const FOUND_FILE: i32 = 1 << SECTION_FILE;
const FOUND_COMPUTED: i32 = 1 << SECTION_COMPUTED;
const FOUND_ANY_TAG: i32 = 1 << SECTION_ANY_TAG;
const FOUND_IFD0: i32 = 1 << SECTION_IFD0;
const FOUND_THUMBNAIL: i32 = 1 << SECTION_THUMBNAIL;
const FOUND_EXIF: i32 = 1 << SECTION_EXIF;
const FOUND_GPS: i32 = 1 << SECTION_GPS;
const FOUND_INTEROP: i32 = 1 << SECTION_INTEROP;
const FOUND_WINXP: i32 = 1 << SECTION_WINXP;
const FOUND_MAKERNOTE: i32 = 1 << SECTION_MAKERNOTE;

// php_image.h's image types.
const IMAGE_FILETYPE_UNKNOWN: i64 = 0;
const IMAGE_FILETYPE_JPEG: i64 = 2;
const IMAGE_FILETYPE_TIFF_II: i64 = 7;
const IMAGE_FILETYPE_TIFF_MM: i64 = 8;
const IMAGE_FILETYPE_HEIF: i64 = 20;

const M_SOF0: u8 = 0xC0;
const M_SOF15: u8 = 0xCF;
const M_DHT: u8 = 0xC4;
const M_DAC: u8 = 0xCC;
const M_SOI: u8 = 0xD8;
const M_EOI: u8 = 0xD9;
const M_SOS: u8 = 0xDA;
const M_EXIF: u8 = 0xE1;
const M_APP12: u8 = 0xEC;
const M_COM: u8 = 0xFE;
/// M_PSEUDO: not a marker byte.
const M_PSEUDO: i32 = 0x123;

fn is_sof(m: u8) -> bool {
    (M_SOF0..=M_SOF15).contains(&m) && m != M_DHT && m != 0xC8 && m != M_DAC
}

// ---------------------------------------------------------------------------
// tag tables
// ---------------------------------------------------------------------------

/// exif_get_tagname.
fn tagname(tag: i64, table: TagTable) -> Option<&'static str> {
    table
        .iter()
        .find(|(t, _)| i64::from(*t) == tag)
        .map(|(_, d)| *d)
}

/// exif_get_tagname_debug.
fn tagname_debug(tag: u32, table: TagTable) -> &'static str {
    tagname(i64::from(tag), table).unwrap_or("UndefinedTag")
}

/// exif_get_tagname_key.
fn tagname_key(tag: u32, table: TagTable) -> Vec<u8> {
    match tagname(i64::from(tag), table) {
        Some(d) => d.as_bytes().to_vec(),
        None => format!("UndefinedTag:0x{tag:04X}").into_bytes(),
    }
}

fn section_name(section: usize) -> &'static str {
    match section {
        SECTION_FILE => "FILE",
        SECTION_COMPUTED => "COMPUTED",
        SECTION_ANY_TAG => "ANY_TAG",
        SECTION_IFD0 => "IFD0",
        SECTION_THUMBNAIL => "THUMBNAIL",
        SECTION_COMMENT => "COMMENT",
        SECTION_APP0 => "APP0",
        SECTION_EXIF => "EXIF",
        SECTION_FPIX => "FPIX",
        SECTION_GPS => "GPS",
        SECTION_INTEROP => "INTEROP",
        SECTION_APP12 => "APP12",
        SECTION_WINXP => "WINXP",
        SECTION_MAKERNOTE => "MAKERNOTE",
        _ => "",
    }
}

fn tag_table(section: usize) -> TagTable {
    match section {
        SECTION_GPS => tags::TAG_TABLE_GPS,
        SECTION_INTEROP => tags::TAG_TABLE_IOP,
        _ => tags::TAG_TABLE_IFD,
    }
}

/// exif_get_sectionlist.
fn section_list(list: i32) -> Vec<u8> {
    let names: Vec<&str> = (0..SECTION_COUNT)
        .filter(|i| list & (1 << i) != 0)
        .map(section_name)
        .collect();
    names.join(", ").into_bytes()
}

#[derive(Clone, Copy, PartialEq)]
enum ByteOrder {
    Intel,
    Motorola,
    Normal,
}

/// maker_note_type: (table, make, id string, offset, byte order, maker
/// offsets). `None` table: custom parsing, which php does not do.
type MakerNote = (
    Option<TagTable>,
    &'static [u8],
    Option<&'static [u8]>,
    usize,
    ByteOrder,
    bool,
);

/// php's maker_note_array.
const MAKER_NOTES: &[MakerNote] = &[
    (
        Some(tags::TAG_TABLE_VND_CANON),
        b"Canon",
        None,
        0,
        ByteOrder::Intel,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_CASIO),
        b"CASIO",
        None,
        0,
        ByteOrder::Motorola,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_FUJI),
        b"FUJIFILM",
        Some(b"FUJIFILM\x0C\x00\x00\x00"),
        12,
        ByteOrder::Intel,
        true,
    ),
    (
        Some(tags::TAG_TABLE_VND_NIKON),
        b"NIKON",
        Some(b"Nikon\x00\x01\x00"),
        8,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_NIKON_990),
        b"NIKON",
        None,
        0,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_OLYMPUS),
        b"OLYMPUS OPTICAL CO.,LTD",
        Some(b"OLYMP\x00\x01\x00"),
        8,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_SAMSUNG),
        b"SAMSUNG",
        None,
        0,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_PANASONIC),
        b"Panasonic",
        Some(b"Panasonic\x00\x00\x00"),
        12,
        ByteOrder::Normal,
        false,
    ),
    (
        None,
        b"DJI",
        Some(b"[ae_dbg_info:"),
        13,
        ByteOrder::Motorola,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_DJI),
        b"DJI",
        None,
        0,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_SONY),
        b"SONY",
        Some(b"SONY DSC \x00\x00\x00"),
        12,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_SONY),
        b"SONY",
        None,
        0,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_PENTAX),
        b"PENTAX",
        Some(b"AOC\x00"),
        6,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_MINOLTA),
        b"Minolta, KONICA MINOLTA",
        None,
        0,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_SIGMA),
        b"SIGMA, FOVEON",
        Some(b"SIGMA\x00\x00\x00"),
        10,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_SIGMA),
        b"SIGMA, FOVEON",
        Some(b"FOVEON\x00\x00\x00"),
        10,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_KYOCERA),
        b"KYOCERA, CONTAX",
        Some(b"KYOCERA            \x00\x00\x00"),
        22,
        ByteOrder::Normal,
        true,
    ),
    (
        Some(tags::TAG_TABLE_VND_RICOH),
        b"RICOH",
        Some(b"Ricoh"),
        5,
        ByteOrder::Motorola,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_RICOH),
        b"RICOH",
        Some(b"RICOH"),
        5,
        ByteOrder::Motorola,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_OLYMPUS),
        b"AGFA",
        Some(b"AGFA \x00\x01"),
        8,
        ByteOrder::Normal,
        false,
    ),
    (
        Some(tags::TAG_TABLE_VND_OLYMPUS),
        b"EPSON",
        Some(b"EPSON\x00\x01\x00"),
        8,
        ByteOrder::Normal,
        false,
    ),
];

// ---------------------------------------------------------------------------
// byte access
// ---------------------------------------------------------------------------

/// `n` bytes at `pos`, zeros past the end.
fn bytes<const N: usize>(m: &[u8], pos: usize) -> [u8; N] {
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        if let Some(&v) = pos.checked_add(i).and_then(|p| m.get(p)) {
            *b = v;
        }
    }
    out
}

fn get16u(m: &[u8], pos: usize, motorola: bool) -> u32 {
    let b = bytes::<2>(m, pos);
    if motorola {
        u32::from(u16::from_be_bytes(b))
    } else {
        u32::from(u16::from_le_bytes(b))
    }
}

fn get16s(m: &[u8], pos: usize, motorola: bool) -> i16 {
    get16u(m, pos, motorola) as u16 as i16
}

fn get32u(m: &[u8], pos: usize, motorola: bool) -> u32 {
    let b = bytes::<4>(m, pos);
    if motorola {
        u32::from_be_bytes(b)
    } else {
        u32::from_le_bytes(b)
    }
}

fn get64u(m: &[u8], pos: usize, motorola: bool) -> u64 {
    let b = bytes::<8>(m, pos);
    if motorola {
        u64::from_be_bytes(b)
    } else {
        u64::from_le_bytes(b)
    }
}

/// php_jpg_get16.
fn jpg_get16(m: &[u8], pos: usize) -> u32 {
    get16u(m, pos, true)
}

fn get_float(m: &[u8], pos: usize) -> f32 {
    f32::from_bits(get32u(m, pos, false))
}

fn get_double(m: &[u8], pos: usize) -> f64 {
    f64::from_bits(get64u(m, pos, false))
}

fn set16u(out: &mut [u8], value: u32, motorola: bool) {
    let v = (value & 0xFFFF) as u16;
    out[..2].copy_from_slice(&if motorola {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    });
}

fn set32u(out: &mut [u8], value: u64, motorola: bool) {
    let v = value as u32;
    out[..4].copy_from_slice(&if motorola {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    });
}

/// zend_strnlen.
fn strnlen(m: &[u8], pos: usize, max: usize) -> usize {
    let end = pos.saturating_add(max).min(m.len());
    let s = m.get(pos..end).unwrap_or(&[]);
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

/// The C string in `b`: up to its first NUL.
fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// exif_convert_any_format.
fn convert_any_format(m: &[u8], pos: usize, format: u32, mi: bool) -> f64 {
    match format as u16 {
        FMT_SBYTE => f64::from(bytes::<1>(m, pos)[0] as i8),
        FMT_BYTE => f64::from(bytes::<1>(m, pos)[0]),
        FMT_USHORT => f64::from(get16u(m, pos, mi)),
        FMT_ULONG => f64::from(get32u(m, pos, mi)),
        FMT_URATIONAL => {
            let den = get32u(m, pos + 4, mi);
            if den == 0 {
                0.0
            } else {
                f64::from(get32u(m, pos, mi)) / f64::from(den)
            }
        }
        FMT_SRATIONAL => {
            let den = get32u(m, pos + 4, mi) as i32;
            if den == 0 {
                0.0
            } else {
                // php_ifd_get32s returns unsigned: the numerator is not
                // sign-extended.
                f64::from(get32u(m, pos, mi)) / f64::from(den)
            }
        }
        FMT_SSHORT => f64::from(get16s(m, pos, mi)),
        FMT_SLONG => f64::from(get32u(m, pos, mi)),
        FMT_SINGLE => f64::from(get_float(m, pos)),
        FMT_DOUBLE => get_double(m, pos),
        _ => 0.0,
    }
}

/// exif_rewrite_tag_format_to_unsigned.
fn format_to_unsigned(format: u32) -> u32 {
    match format as u16 {
        FMT_SBYTE => u32::from(FMT_BYTE),
        FMT_SRATIONAL => u32::from(FMT_URATIONAL),
        FMT_SSHORT => u32::from(FMT_USHORT),
        FMT_SLONG => u32::from(FMT_ULONG),
        _ => format,
    }
}

fn float_to_size(x: f64) -> usize {
    if x < 0.0 || x.is_nan() {
        0
    } else if x > usize::MAX as f64 {
        usize::MAX
    } else {
        x as usize
    }
}

/// exif_convert_any_to_int (size_t).
fn convert_any_to_int(m: &[u8], pos: usize, format: u32, mi: bool) -> usize {
    match format as u16 {
        FMT_SBYTE => bytes::<1>(m, pos)[0] as i8 as isize as usize,
        FMT_BYTE => usize::from(bytes::<1>(m, pos)[0]),
        FMT_USHORT | FMT_SSHORT => get16u(m, pos, mi) as usize,
        FMT_ULONG | FMT_SLONG => get32u(m, pos, mi) as usize,
        FMT_URATIONAL => get32u(m, pos, mi)
            .checked_div(get32u(m, pos + 4, mi))
            .unwrap_or(0) as usize,
        FMT_SRATIONAL => {
            let num = get32u(m, pos, mi) as i32;
            let den = get32u(m, pos + 4, mi) as i32;
            if den == 0 {
                0
            } else if num == i32::MIN && den == -1 {
                i32::MAX as usize
            } else {
                (num / den) as isize as usize
            }
        }
        FMT_SINGLE => float_to_size(f64::from(get_float(m, pos))),
        FMT_DOUBLE => float_to_size(get_double(m, pos)),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// image info
// ---------------------------------------------------------------------------

/// image_info_value's numeric members, as the union's 8 bytes (little
/// endian: `u`/`i`/`f` and a rational's numerator in the low half).
#[derive(Clone, Copy, Default)]
struct Num(u64);

impl Num {
    fn u(self) -> u32 {
        self.0 as u32
    }
    fn i(self) -> i32 {
        self.0 as u32 as i32
    }
    fn f(self) -> f32 {
        f32::from_bits(self.0 as u32)
    }
    fn d(self) -> f64 {
        f64::from_bits(self.0)
    }
    fn num_den(self) -> (u32, u32) {
        (self.0 as u32, (self.0 >> 32) as u32)
    }
    fn pair(num: u32, den: u32) -> Num {
        Num(u64::from(num) | (u64::from(den) << 32))
    }
}

enum Payload {
    None,
    Str(Vec<u8>),
    Nums(Vec<Num>),
}

/// image_info_data.
struct InfoData {
    tag: u16,
    format: u16,
    length: u32,
    name: Vec<u8>,
    value: Payload,
}

#[derive(Default)]
struct Thumbnail {
    filetype: i64,
    width: usize,
    height: usize,
    size: usize,
    offset: usize,
    data: Option<Vec<u8>>,
}

/// A position in a buffer.
#[derive(Clone)]
struct Ptr {
    mem: Rc<[u8]>,
    pos: usize,
}

/// exif_offset_info: offsets count from `base` (an index into `mem`,
/// possibly before its start); `[start, end)` may be read.
#[derive(Clone)]
struct OffsetInfo {
    mem: Rc<[u8]>,
    base: i64,
    start: usize,
    end: usize,
}

impl OffsetInfo {
    fn new(mem: Rc<[u8]>, base: i64, start: usize, len: usize) -> OffsetInfo {
        OffsetInfo {
            mem,
            base,
            start,
            end: start + len,
        }
    }

    /// exif_offset_info_try_get.
    fn try_get(&self, offset: u64, len: u64) -> Option<usize> {
        let s = i128::from(self.base) + i128::from(offset);
        let e = s + i128::from(len);
        (s >= self.start as i128 && e <= self.end as i128).then_some(s as usize)
    }

    /// exif_offset_info_contains.
    fn contains(&self, pos: usize, len: usize) -> bool {
        pos >= self.start && pos.checked_add(len).is_some_and(|e| e <= self.end)
    }
}

/// The file being read: a stream resource and the host that owns it.
struct Infile<'h> {
    host: &'h mut dyn Host,
    res: Value,
}

impl Infile<'_> {
    fn seek(&mut self, pos: i64) -> bool {
        self.host.stream_seek(&self.res, pos, libc::SEEK_SET)
    }
    fn seek_end(&mut self) -> bool {
        self.host.stream_seek(&self.res, 0, libc::SEEK_END)
    }
    fn tell(&mut self) -> i64 {
        self.host.stream_tell(&self.res).unwrap_or(-1)
    }
    /// exif_read_from_stream_file_looped: up to `n` bytes.
    fn read(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        while out.len() < n {
            match self.host.stream_read(&self.res, (n - out.len()) as i64) {
                Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                _ => break,
            }
        }
        out
    }
    fn getc(&mut self) -> Option<u8> {
        self.read(1).first().copied()
    }
}

/// image_info_type.
struct ImageInfo<'h> {
    infile: Infile<'h>,
    file_name: Option<Vec<u8>>,
    file_date_time: i64,
    file_size: usize,
    file_type: i64,
    height: i32,
    width: i32,
    is_color: i32,
    make: Option<Vec<u8>>,
    model: Option<Vec<u8>>,
    aperture_fnumber: f32,
    exposure_time: f32,
    focalplane_units: f64,
    ccd_width: f32,
    focalplane_x_res: f64,
    exif_image_width: usize,
    distance: f32,
    /// 1 Motorola, 0 Intel, -1 not known yet.
    motorola_intel: i32,
    user_comment: Option<Vec<u8>>,
    user_comment_length: i32,
    user_comment_encoding: Option<Vec<u8>>,
    encode_unicode: Vec<u8>,
    decode_unicode_be: Vec<u8>,
    decode_unicode_le: Vec<u8>,
    encode_jis: Vec<u8>,
    decode_jis_be: Vec<u8>,
    decode_jis_le: Vec<u8>,
    copyright: Option<Vec<u8>>,
    copyright_photographer: Option<Vec<u8>>,
    copyright_editor: Option<Vec<u8>>,
    /// (tag, value) of the WinXP fields.
    xp_fields: Vec<(u32, Vec<u8>)>,
    thumbnail: Thumbnail,
    sections_found: i32,
    info_list: [Vec<InfoData>; SECTION_COUNT],
    read_thumbnail: bool,
    ifd_nesting_level: i32,
    ifd_count: i32,
    num_errors: i32,
    /// The diagnostics raised, in order: (level, the file name php puts
    /// in the call's parentheses, message).
    diags: Vec<(i64, Vec<u8>, Vec<u8>)>,
    /// The JPEG sections read (php keeps them until the end).
    sections: Vec<Rc<[u8]>>,
}

fn ini_str(name: &[u8]) -> Vec<u8> {
    crate::main::ini::ini_value(name)
        .flatten()
        .unwrap_or_default()
}

impl<'h> ImageInfo<'h> {
    fn new(infile: Infile<'h>) -> ImageInfo<'h> {
        ImageInfo {
            infile,
            file_name: None,
            file_date_time: 0,
            file_size: 0,
            file_type: IMAGE_FILETYPE_UNKNOWN,
            height: 0,
            width: 0,
            is_color: 0,
            make: None,
            model: None,
            aperture_fnumber: 0.0,
            exposure_time: 0.0,
            focalplane_units: 0.0,
            ccd_width: 0.0,
            focalplane_x_res: 0.0,
            exif_image_width: 0,
            distance: 0.0,
            motorola_intel: -1,
            user_comment: None,
            user_comment_length: 0,
            user_comment_encoding: None,
            encode_unicode: ini_str(b"exif.encode_unicode"),
            decode_unicode_be: ini_str(b"exif.decode_unicode_motorola"),
            decode_unicode_le: ini_str(b"exif.decode_unicode_intel"),
            encode_jis: ini_str(b"exif.encode_jis"),
            decode_jis_be: ini_str(b"exif.decode_jis_motorola"),
            decode_jis_le: ini_str(b"exif.decode_jis_intel"),
            copyright: None,
            copyright_photographer: None,
            copyright_editor: None,
            xp_fields: Vec::new(),
            thumbnail: Thumbnail::default(),
            sections_found: 0,
            info_list: Default::default(),
            read_thumbnail: false,
            ifd_nesting_level: 0,
            ifd_count: 0,
            num_errors: 0,
            diags: Vec::new(),
            sections: Vec::new(),
        }
    }

    fn mi(&self) -> bool {
        self.motorola_intel == 1
    }

    /// exif_error_docref: at most EXIF_MAX_ERRORS per file.
    fn error(&mut self, level: i64, msg: impl Into<Vec<u8>>) {
        self.num_errors += 1;
        if self.num_errors > EXIF_MAX_ERRORS {
            if self.num_errors == EXIF_MAX_ERRORS + 1 {
                self.diags.push((
                    level,
                    Vec::new(),
                    b"Further exif parsing errors have been suppressed".to_vec(),
                ));
            }
            return;
        }
        let file = self.file_name.clone().unwrap_or_default();
        self.diags.push((level, file, msg.into()));
    }

    fn warn(&mut self, msg: impl Into<Vec<u8>>) {
        self.error(E_WARNING, msg);
    }

    // --- image_info lists -------------------------------------------------

    /// exif_iif_add_value: `value` holds at least `value_len` bytes from
    /// `pos` (None: no data).
    #[allow(clippy::too_many_arguments)]
    fn add_value(
        &mut self,
        section: usize,
        name: &[u8],
        tag: u16,
        format: u16,
        length: usize,
        value: Option<(&[u8], usize)>,
        value_len: usize,
        mi: bool,
    ) {
        let mut data = InfoData {
            tag,
            format,
            length: length as u32,
            name: name.to_vec(),
            value: Payload::None,
        };
        let mut value = value;
        match format {
            FMT_STRING => {
                if length > value_len {
                    self.warn(format!(
                        "length > value_len: {} > {value_len}",
                        length as i32
                    ));
                    value = None;
                }
                match value {
                    Some((m, pos)) => {
                        let n = strnlen(m, pos, length);
                        data.value = Payload::Str(m[pos..pos + n].to_vec());
                        data.length = n as u32;
                    }
                    None => {
                        data.length = 0;
                        data.value = Payload::Str(Vec::new());
                    }
                }
            }
            FMT_USHORT | FMT_ULONG | FMT_URATIONAL | FMT_SSHORT | FMT_SLONG | FMT_SRATIONAL
            | FMT_SINGLE | FMT_DOUBLE => {
                if length > 0 {
                    let mut list = vec![Num::default(); length];
                    let step = BYTES_PER_FORMAT[format as usize];
                    if let Some((m, pos)) = value {
                        for (idx, slot) in list.iter_mut().enumerate() {
                            let off = idx * step;
                            if value_len.saturating_sub(off) < step {
                                self.warn("Value too short");
                                break;
                            }
                            let p = pos + off;
                            *slot = match format {
                                FMT_USHORT => Num(u64::from(get16u(m, p, mi))),
                                FMT_ULONG => Num(u64::from(get32u(m, p, mi))),
                                FMT_URATIONAL | FMT_SRATIONAL => {
                                    Num::pair(get32u(m, p, mi), get32u(m, p + 4, mi))
                                }
                                FMT_SSHORT => Num(u64::from(get16s(m, p, mi) as i32 as u32)),
                                FMT_SLONG => Num(u64::from(get32u(m, p, mi))),
                                FMT_SINGLE => Num(u64::from(get_float(m, p).to_bits())),
                                _ => Num(get_double(m, p).to_bits()),
                            };
                        }
                    }
                    data.value = Payload::Nums(list);
                }
            }
            _ => {
                if !matches!(format, FMT_SBYTE | FMT_BYTE | FMT_UNDEFINED) {
                    // php sets the tag, not the format.
                    data.tag = FMT_UNDEFINED;
                }
                if length > 0 || format == FMT_UNDEFINED {
                    if length > value_len {
                        self.warn(format!(
                            "length > value_len: {} > {value_len}",
                            length as i32
                        ));
                        value = None;
                    }
                    match value {
                        Some((m, pos)) => {
                            let n = if u32::from(tag) == TAG_MAKER_NOTE {
                                strnlen(m, pos, length)
                            } else {
                                length
                            };
                            let mut s = m.get(pos..).unwrap_or(&[]).to_vec();
                            s.resize(n, 0);
                            data.value = Payload::Str(s);
                            data.length = n as u32;
                        }
                        None => {
                            data.length = 0;
                            data.value = Payload::Str(Vec::new());
                        }
                    }
                }
            }
        }
        self.info_list[section].push(data);
        self.sections_found |= 1 << section;
    }

    /// exif_iif_add_tag.
    #[allow(clippy::too_many_arguments)]
    fn add_tag(
        &mut self,
        section: usize,
        name: &[u8],
        tag: u16,
        format: u16,
        length: usize,
        value: Option<(&[u8], usize)>,
        value_len: usize,
    ) {
        let mi = self.mi();
        self.add_value(section, name, tag, format, length, value, value_len, mi);
    }
}

// ---------------------------------------------------------------------------
// parsing
// ---------------------------------------------------------------------------

/// What a comment conversion gives: the zend_multibyte hooks mbstring
/// installs (None: an encoding mbstring does not know).
fn mb_convert(src: &[u8], to: &[u8], from: &[u8]) -> Option<Vec<u8>> {
    #[cfg(feature = "mbstring-libmbfl")]
    {
        crate::ext::mbstring::zend_convert(src, cstr(to), cstr(from))
    }
    #[cfg(not(feature = "mbstring-libmbfl"))]
    {
        let _ = (src, to, from);
        None
    }
}

/// What exif_process_user_comment gives: the comment, its encoding, its
/// length, and the value with the trailing spaces php blanks out in place.
type UserComment = (Option<Vec<u8>>, Option<Vec<u8>>, i32, Vec<u8>);

/// exif_process_string_raw: the bytes and a NUL, and that length.
fn process_string_raw(value: &[u8]) -> (Option<Vec<u8>>, usize) {
    if value.is_empty() {
        return (None, 0);
    }
    let mut v = value.to_vec();
    v.push(0);
    (Some(v), value.len() + 1)
}

/// The thumbnail's SOFn: (height, width).
fn sof_size(m: &[u8], pos: usize) -> (usize, usize) {
    (
        jpg_get16(m, pos + 3) as usize,
        jpg_get16(m, pos + 5) as usize,
    )
}

impl ImageInfo<'_> {
    /// exif_process_user_comment: the comment, its encoding, its length,
    /// and the value with the trailing spaces php blanks out in place.
    fn process_user_comment(&mut self, value: &[u8]) -> UserComment {
        let mut v = value.to_vec();
        let mut start = 0usize;
        let mut encoding = None;
        if v.len() >= 8 {
            let head = &v[..8];
            if head == b"UNICODE\0" {
                encoding = Some(b"UNICODE".to_vec());
                start = 8;
                let mut decode = if self.mi() {
                    self.decode_unicode_be.clone()
                } else {
                    self.decode_unicode_le.clone()
                };
                if v.len() - start >= 2 && v[start..start + 2] == *b"\xFE\xFF" {
                    decode = b"UCS-2BE".to_vec();
                    start += 2;
                } else if v.len() - start >= 2 && v[start..start + 2] == *b"\xFF\xFE" {
                    decode = b"UCS-2LE".to_vec();
                    start += 2;
                }
                let body = &v[start..];
                return match mb_convert(body, &self.encode_unicode, &decode) {
                    Some(mut c) => {
                        let n = c.len() as i32;
                        c.push(0);
                        (Some(c), encoding, n, v)
                    }
                    None => {
                        let (s, n) = process_string_raw(body);
                        (s, encoding, n as i32, v)
                    }
                };
            } else if head == b"ASCII\0\0\0" {
                encoding = Some(b"ASCII".to_vec());
                start = 8;
            } else if head == b"JIS\0\0\0\0\0" {
                encoding = Some(b"JIS".to_vec());
                start = 8;
                let decode = if self.mi() {
                    self.decode_jis_be.clone()
                } else {
                    self.decode_jis_le.clone()
                };
                let body = &v[start..];
                return match mb_convert(body, &self.encode_jis, &decode) {
                    Some(mut c) => {
                        let n = c.len() as i32;
                        c.push(0);
                        (Some(c), encoding, n, v)
                    }
                    None => {
                        let (s, n) = process_string_raw(body);
                        (s, encoding, n as i32, v)
                    }
                };
            } else if head == [0u8; 8] {
                encoding = Some(b"UNDEFINED".to_vec());
                start = 8;
            }
        }
        // Olympus pads with spaces: php blanks them out in the value.
        let count = v.len() - start;
        if count > 0 {
            let mut a = count - 1;
            while a > 0 && v[start + a] == b' ' {
                v[start + a] = 0;
                a -= 1;
            }
        }
        // exif_process_string, then strlen.
        let body = &v[start..];
        let n = body.iter().position(|&c| c == 0).unwrap_or(body.len());
        let s = body[..n].to_vec();
        let len = s.len() as i32;
        (Some(s), encoding, len, v)
    }

    /// exif_process_unicode.
    fn process_unicode(&mut self, tag: u32, value: &[u8]) {
        let decode = if self.mi() {
            self.decode_unicode_be.clone()
        } else {
            self.decode_unicode_le.clone()
        };
        let v = match mb_convert(value, &self.encode_unicode, &decode) {
            Some(c) => c,
            None => process_string_raw(value).0.unwrap_or_default(),
        };
        self.xp_fields.push((tag, v));
    }

    /// exif_process_IFD_in_MAKERNOTE.
    fn process_ifd_in_makernote(
        &mut self,
        value: &Ptr,
        value_len: i32,
        info: &OffsetInfo,
        displacement: usize,
    ) -> bool {
        let vb = &value.mem[value.pos.min(value.mem.len())..];
        let mut found = None;
        for mn in MAKER_NOTES {
            if self.make.as_deref().map(cstr) != Some(mn.1) {
                continue;
            }
            if let Some(id) = mn.2 {
                if value_len >= id.len() as i32 && !strncmp_eq(id, vb, id.len()) {
                    continue;
                }
            }
            found = Some(mn);
            break;
        }
        // Unknown maker: the note stays a string.
        let Some(&(table, _, _, offset, order, maker_offsets)) = found else {
            return true;
        };
        if value_len < 2 || offset as i32 >= value_len - 1 {
            self.warn(format!(
                "IFD data too short: 0x{:04X} offset 0x{:04X}",
                value_len, offset
            ));
            return true;
        }
        let Some(table) = table else {
            return true;
        };
        let dir_start = value.pos + offset;
        self.sections_found |= FOUND_MAKERNOTE;
        let old_mi = self.motorola_intel;
        match order {
            ByteOrder::Intel => self.motorola_intel = 0,
            ByteOrder::Motorola => self.motorola_intel = 1,
            ByteOrder::Normal => {}
        }
        let mut n = get16u(&value.mem, dir_start, self.mi()) as i32;
        if 2 + n * 12 > value_len {
            self.error(
                E_NOTICE,
                "Potentially invalid endianness, trying again with different endianness before imminent failure.",
            );
            self.motorola_intel = if self.motorola_intel == 0 { 1 } else { 0 };
            n = get16u(&value.mem, dir_start, self.mi()) as i32;
        }
        if 2 + n * 12 > value_len {
            self.warn(format!(
                "Illegal IFD size: 2 + 0x{:04X}*12 = 0x{:04X} > 0x{:04X}",
                n,
                2 + n * 12,
                value_len
            ));
            return false;
        }
        if offset as i32 > value_len - (2 + n * 12) {
            self.warn(format!(
                "Illegal IFD size: 0x{:04X} > 0x{:04X}",
                offset as i32 + 2 + n * 12,
                value_len
            ));
            return false;
        }
        let maker_info;
        let info = if maker_offsets {
            maker_info = OffsetInfo::new(
                value.mem.clone(),
                value.pos as i64,
                value.pos,
                value_len as usize,
            );
            &maker_info
        } else {
            info
        };
        for de in 0..n as usize {
            let entry = Ptr {
                mem: value.mem.clone(),
                pos: dir_start + 2 + 12 * de,
            };
            if !self.process_ifd_tag(&entry, info, displacement, SECTION_MAKERNOTE, false, table) {
                return false;
            }
        }
        self.motorola_intel = old_mi;
        true
    }

    /// exif_process_IFD_TAG.
    fn process_ifd_tag(
        &mut self,
        entry: &Ptr,
        info: &OffsetInfo,
        displacement: usize,
        section: usize,
        read_next: bool,
        table: TagTable,
    ) -> bool {
        let count = self.ifd_count;
        self.ifd_count += 1;
        if count > MAX_IFD_TAGS {
            self.warn("corrupt EXIF header: maximum IFD tag count reached");
            return false;
        }
        if self.ifd_nesting_level > MAX_IFD_NESTING_LEVEL {
            self.warn("corrupt EXIF header: maximum directory nesting level reached");
            return false;
        }
        // Each tag may keep up to FileSize bytes.
        if rhp_values::mem::has_oom() {
            return false;
        }
        self.ifd_nesting_level += 1;
        let r = self.process_ifd_tag_impl(entry, info, displacement, section, read_next, table);
        self.ifd_nesting_level -= 1;
        r
    }

    fn process_ifd_tag_impl(
        &mut self,
        entry: &Ptr,
        info: &OffsetInfo,
        displacement: usize,
        section: usize,
        read_next: bool,
        table: TagTable,
    ) -> bool {
        let mi = self.mi();
        let em = &entry.mem;
        let tag = get16u(em, entry.pos, mi);
        let mut format = get16u(em, entry.pos + 2, mi);
        let components = get32u(em, entry.pos + 4, mi);
        if format == 0 || format > NUM_FORMATS {
            self.warn(format!(
                "Process tag(x{tag:04X}={}): Illegal format code 0x{format:04X}, suppose BYTE",
                tagname_debug(tag, table)
            ));
            format = u32::from(FMT_BYTE);
        }
        let byte_count_signed = i64::from(components) * BYTES_PER_FORMAT[format as usize] as i64;
        if byte_count_signed > i64::from(i32::MAX) {
            self.warn(format!(
                "Process tag(x{tag:04X}={}): Illegal byte_count",
                tagname_debug(tag, table)
            ));
            return false;
        }
        let byte_count = byte_count_signed as usize;
        let value = if byte_count > 4 {
            let offset_val = get32u(em, entry.pos + 8, mi) as usize;
            match info.try_get(offset_val as u64, byte_count as u64) {
                Some(pos) => Ptr {
                    mem: info.mem.clone(),
                    pos,
                },
                None => {
                    // JPEG offsets count from the TIFF header in APP1; TIFF
                    // ones from the file's start.
                    let fs = self.file_size;
                    if byte_count > fs
                        || offset_val > fs - byte_count
                        || !matches!(
                            self.file_type,
                            IMAGE_FILETYPE_TIFF_II | IMAGE_FILETYPE_TIFF_MM | IMAGE_FILETYPE_JPEG
                        )
                    {
                        self.warn(format!(
                            "Process tag(x{tag:04X}={}): Illegal pointer offset(x{:04X} + x{:04X} = x{:04X} > x{:04X})",
                            tagname_debug(tag, table),
                            offset_val as u32,
                            byte_count as u32,
                            (offset_val + byte_count) as u32,
                            fs as u32
                        ));
                        return false;
                    }
                    let fpos = self.infile.tell();
                    let target = (displacement + offset_val) as i64;
                    self.infile.seek(target);
                    let fgot = self.infile.tell();
                    if fgot != target {
                        self.warn(format!(
                            "Wrong file pointer: 0x{:08X} != 0x{:08X}",
                            fgot as u32, target as u32
                        ));
                        return false;
                    }
                    let mut buf = self.infile.read(byte_count);
                    self.infile.seek(fpos);
                    if buf.len() != byte_count {
                        self.warn(ERROR_FILEEOF);
                        return false;
                    }
                    // php's 32-byte stack buffer for short values.
                    if buf.len() < 32 {
                        buf.resize(32, 0);
                    }
                    Ptr {
                        mem: Rc::from(buf),
                        pos: 0,
                    }
                }
            }
        } else {
            Ptr {
                mem: entry.mem.clone(),
                pos: entry.pos + 8,
            }
        };
        let vm = value.mem.clone();
        let vp = value.pos;
        self.sections_found |= FOUND_ANY_TAG;

        macro_rules! require_non_empty {
            () => {
                if byte_count == 0 {
                    self.warn(format!(
                        "Process tag(x{tag:04X}={}): Cannot be empty",
                        tagname_debug(tag, table)
                    ));
                    return false;
                }
            };
        }

        // The value as php stores it (UserComment's trailing spaces
        // blanked out).
        let mut stored: Option<Vec<u8>> = None;
        if section == SECTION_THUMBNAIL {
            if self.thumbnail.data.is_none() {
                require_non_empty!();
                let uf = format_to_unsigned(format);
                match tag {
                    TAG_NEW_SUBFILE_IMAGEWIDTH | TAG_COMP_IMAGE_WIDTH => {
                        self.thumbnail.width = convert_any_to_int(&vm, vp, uf, mi);
                    }
                    TAG_IMAGEHEIGHT | TAG_COMP_IMAGE_HEIGHT => {
                        self.thumbnail.height = convert_any_to_int(&vm, vp, uf, mi);
                    }
                    TAG_STRIP_OFFSETS | TAG_JPEG_INTERCHANGE_FORMAT => {
                        self.thumbnail.offset = convert_any_to_int(&vm, vp, uf, mi);
                    }
                    TAG_STRIP_BYTE_COUNTS => {
                        self.thumbnail.filetype = if matches!(
                            self.file_type,
                            IMAGE_FILETYPE_TIFF_II | IMAGE_FILETYPE_TIFF_MM
                        ) {
                            self.file_type
                        } else {
                            IMAGE_FILETYPE_TIFF_MM
                        };
                        self.thumbnail.size = convert_any_to_int(&vm, vp, uf, mi);
                    }
                    TAG_JPEG_INTERCHANGE_FORMAT_LEN
                        if self.thumbnail.filetype == IMAGE_FILETYPE_UNKNOWN =>
                    {
                        self.thumbnail.filetype = IMAGE_FILETYPE_JPEG;
                        self.thumbnail.size = convert_any_to_int(&vm, vp, uf, mi);
                    }
                    _ => {}
                }
            }
        } else if section == SECTION_IFD0 || section == SECTION_EXIF {
            let vbytes = |n: usize| -> Vec<u8> {
                let mut b = vm.get(vp..).unwrap_or(&[]).to_vec();
                b.resize(n, 0);
                b
            };
            match tag {
                TAG_COPYRIGHT => {
                    if byte_count > 1 {
                        let v = vbytes(byte_count);
                        let length = strnlen(&v, 0, byte_count);
                        if length > 0 {
                            if length < byte_count - 1 {
                                // "<photographer> NUL <editor> NUL"
                                let photographer = v[..length].to_vec();
                                let editor = cstr(&v[length + 1..]).to_vec();
                                let mut c = photographer.clone();
                                c.extend_from_slice(b", ");
                                c.extend_from_slice(&editor);
                                self.copyright_photographer = Some(photographer);
                                self.copyright_editor = Some(editor);
                                self.copyright = Some(c);
                            } else {
                                self.copyright = Some(cstr(&v).to_vec());
                            }
                        }
                    }
                }
                TAG_USERCOMMENT => {
                    let v = vbytes(byte_count);
                    let (c, e, l, blanked) = self.process_user_comment(&v);
                    self.user_comment = c;
                    self.user_comment_encoding = e;
                    self.user_comment_length = l;
                    stored = Some(blanked);
                }
                TAG_XP_TITLE..=TAG_XP_SUBJECT => {
                    self.sections_found |= FOUND_WINXP;
                    let v = vbytes(byte_count);
                    self.process_unicode(tag, &v);
                }
                TAG_FNUMBER => {
                    require_non_empty!();
                    self.aperture_fnumber = convert_any_format(&vm, vp, format, mi) as f32;
                }
                TAG_APERTURE | TAG_MAX_APERTURE => {
                    if self.aperture_fnumber == 0.0 {
                        require_non_empty!();
                        let x = convert_any_format(&vm, vp, format, mi)
                            * f64::from(std::f32::consts::LN_2)
                            * 0.5;
                        self.aperture_fnumber = (x as f32).exp();
                    }
                }
                TAG_SHUTTERSPEED => {
                    if self.exposure_time == 0.0 {
                        require_non_empty!();
                        let x = -convert_any_format(&vm, vp, format, mi)
                            * f64::from(std::f32::consts::LN_2);
                        self.exposure_time = (x as f32).exp();
                    }
                }
                TAG_EXPOSURETIME => self.exposure_time = -1.0,
                TAG_COMP_IMAGE_WIDTH => {
                    require_non_empty!();
                    self.exif_image_width =
                        convert_any_to_int(&vm, vp, format_to_unsigned(format), mi);
                }
                TAG_FOCALPLANE_X_RES => {
                    require_non_empty!();
                    self.focalplane_x_res = convert_any_format(&vm, vp, format, mi);
                }
                TAG_SUBJECT_DISTANCE => {
                    require_non_empty!();
                    self.distance = convert_any_format(&vm, vp, format, mi) as f32;
                }
                TAG_FOCALPLANE_RESOLUTION_UNIT => {
                    require_non_empty!();
                    match convert_any_to_int(&vm, vp, format, mi) {
                        1 | 2 => self.focalplane_units = 25.4,
                        3 => self.focalplane_units = 10.0,
                        4 => self.focalplane_units = 1.0,
                        5 => self.focalplane_units = 0.001,
                        _ => {}
                    }
                }
                TAG_SUB_IFD => {
                    if format == u32::from(FMT_IFD) {
                        self.error(E_NOTICE, "Skip SUB IFD");
                    }
                }
                TAG_MAKE => self.make = Some(cstr(&vbytes(byte_count)).to_vec()),
                TAG_MODEL => self.model = Some(cstr(&vbytes(byte_count)).to_vec()),
                TAG_MAKER_NOTE => {
                    if !self.process_ifd_in_makernote(&value, byte_count as i32, info, displacement)
                    {
                        return false;
                    }
                }
                TAG_EXIF_IFD_POINTER | TAG_GPS_IFD_POINTER | TAG_INTEROP_IFD_POINTER
                    if read_next =>
                {
                    require_non_empty!();
                    let sub = match tag {
                        TAG_EXIF_IFD_POINTER => {
                            self.sections_found |= FOUND_EXIF;
                            SECTION_EXIF
                        }
                        TAG_GPS_IFD_POINTER => {
                            self.sections_found |= FOUND_GPS;
                            SECTION_GPS
                        }
                        _ => {
                            self.sections_found |= FOUND_INTEROP;
                            SECTION_INTEROP
                        }
                    };
                    let off = get32u(&vm, vp, self.mi());
                    let Some(sub_start) = info.try_get(u64::from(off), 0) else {
                        self.warn("Illegal IFD Pointer");
                        return false;
                    };
                    if !self.process_ifd_in_jpeg(sub_start, info, displacement, sub, tag) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        let name = tagname_key(tag, table);
        match &stored {
            Some(v) => self.add_tag(
                section,
                &name,
                tag as u16,
                format as u16,
                components as usize,
                Some((v, 0)),
                byte_count,
            ),
            None => self.add_tag(
                section,
                &name,
                tag as u16,
                format as u16,
                components as usize,
                Some((&vm, vp)),
                byte_count,
            ),
        }
        true
    }

    /// exif_process_IFD_in_JPEG: the directory at `dir_start` in
    /// `info.mem`.
    fn process_ifd_in_jpeg(
        &mut self,
        dir_start: usize,
        info: &OffsetInfo,
        displacement: usize,
        section: usize,
        tag: u32,
    ) -> bool {
        self.sections_found |= FOUND_IFD0;
        if !info.contains(dir_start, 2) {
            self.warn("Illegal IFD size");
            return false;
        }
        let n = get16u(&info.mem, dir_start, self.mi()) as usize;
        if !info.contains(dir_start + 2, n * 12) {
            self.warn(format!(
                "Illegal IFD size: x{:04X} + 2 + x{:04X}*12 = x{:04X} > x{:04X}",
                (dir_start + 2 - info.start) as i32,
                n,
                (dir_start + 2 + n * 12 - info.start) as i32,
                (info.end - info.start) as u32
            ));
            return false;
        }
        for de in 0..n {
            let entry = Ptr {
                mem: info.mem.clone(),
                pos: dir_start + 2 + 12 * de,
            };
            if !self.process_ifd_tag(
                &entry,
                info,
                displacement,
                section,
                true,
                tag_table(section),
            ) {
                return false;
            }
        }
        // IFD2 is ignored.
        if section == SECTION_THUMBNAIL {
            return true;
        }
        // No next-IFD offset after the entries: no next IFD.
        if !info.contains(dir_start + 2 + n * 12, 4) {
            return true;
        }
        let next = if tag != TAG_EXIF_IFD_POINTER && tag != TAG_GPS_IFD_POINTER {
            get32u(&info.mem, dir_start + 2 + 12 * n, self.mi()) as i32
        } else {
            0
        };
        if next != 0 {
            // An int passed as size_t.
            let Some(next_start) = info.try_get(next as i64 as u64, 0) else {
                self.warn("Illegal IFD offset");
                return false;
            };
            // IFD1: the thumbnail's.
            if self.process_ifd_in_jpeg(next_start, info, displacement, SECTION_THUMBNAIL, 0) {
                if self.thumbnail.filetype != IMAGE_FILETYPE_UNKNOWN
                    && self.thumbnail.size != 0
                    && self.thumbnail.offset != 0
                    && self.read_thumbnail
                {
                    self.thumbnail_extract(info);
                }
                return true;
            }
            return false;
        }
        true
    }

    /// exif_process_TIFF_in_JPEG.
    fn process_tiff_in_jpeg(
        &mut self,
        mem: &Rc<[u8]>,
        start: usize,
        length: usize,
        displacement: usize,
    ) {
        if length < 2 {
            self.warn("Missing TIFF alignment marker");
            return;
        }
        match &mem[start..start + 2] {
            b"II" => self.motorola_intel = 0,
            b"MM" => self.motorola_intel = 1,
            _ => {
                self.warn("Invalid TIFF alignment marker");
                return;
            }
        }
        if length < 8 {
            self.warn("Invalid TIFF start (1)");
            return;
        }
        let v2a = get16u(mem, start + 2, self.mi());
        let offset_of_ifd = get32u(mem, start + 4, self.mi()) as usize;
        if v2a != 0x2a || offset_of_ifd < 0x08 {
            self.warn("Invalid TIFF start (1)");
            return;
        }
        if offset_of_ifd > length {
            self.warn("Invalid IFD start");
            return;
        }
        self.sections_found |= FOUND_IFD0;
        let info = OffsetInfo::new(mem.clone(), start as i64, start, length);
        self.process_ifd_in_jpeg(start + offset_of_ifd, &info, displacement, SECTION_IFD0, 0);
        // The CCD width, in millimeters.
        if self.focalplane_x_res != 0.0 {
            self.ccd_width = (self.exif_image_width as f64 * self.focalplane_units
                / self.focalplane_x_res) as f32;
        }
    }

    /// exif_process_APP1.
    fn process_app1(&mut self, mem: &Rc<[u8]>, length: usize, displacement: usize) {
        if length <= 8 || mem[2..8] != *b"Exif\0\0" {
            self.warn("Incorrect APP1 Exif Identifier Code");
            return;
        }
        self.process_tiff_in_jpeg(mem, 8, length - 8, displacement + 8);
    }

    /// exif_process_APP12.
    fn process_app12(&mut self, buf: &[u8], length: usize) {
        let l1 = strnlen(buf, 2, length - 2);
        if l1 > 0 {
            self.add_tag(
                SECTION_APP12,
                b"Company",
                TAG_NONE,
                FMT_STRING,
                l1,
                Some((buf, 2)),
                l1,
            );
            if length > 2 + l1 + 1 {
                let l2 = strnlen(buf, 2 + l1 + 1, length - 2 - l1 - 1);
                self.add_tag(
                    SECTION_APP12,
                    b"Info",
                    TAG_NONE,
                    FMT_STRING,
                    l2,
                    Some((buf, 2 + l1 + 1)),
                    l2,
                );
            }
        }
    }

    /// exif_scan_JPEG_header: the markers up to SOS or EOI.
    fn scan_jpeg_header(&mut self) -> bool {
        let mut last_marker = M_PSEUDO;
        let mut comment_correction = 1;
        loop {
            // Every section is kept: stop once over the limit.
            if rhp_values::mem::has_oom() {
                return false;
            }
            if last_marker == i32::from(M_COM) && comment_correction != 0 {
                comment_correction = 2;
            }
            let mut marker;
            loop {
                let Some(c) = self.infile.getc() else {
                    self.warn(ERROR_CORRUPT);
                    return false;
                };
                marker = c;
                if last_marker == i32::from(M_COM) && comment_correction > 0 {
                    if marker != 0xFF {
                        marker = 0xFF;
                        comment_correction -= 1;
                    } else {
                        // Stop skipping for COM.
                        last_marker = M_PSEUDO;
                    }
                }
                if marker != 0xFF {
                    break;
                }
            }
            if last_marker == i32::from(M_COM) && comment_correction == 0 {
                self.error(
                    E_NOTICE,
                    "Image has corrupt COM section: some software set wrong length information",
                );
            }
            if last_marker == i32::from(M_COM) && comment_correction != 0 {
                // php returns M_EOI, which is true.
                return true;
            }
            let fpos = self.infile.tell().max(0) as usize;
            let Some(lh) = self.infile.getc() else {
                self.warn(ERROR_CORRUPT);
                return false;
            };
            let Some(ll) = self.infile.getc() else {
                self.warn(ERROR_CORRUPT);
                return false;
            };
            let itemlen = (usize::from(lh) << 8) | usize::from(ll);
            if itemlen < 2 {
                self.warn(ERROR_CORRUPT);
                return false;
            }
            let mut data = vec![lh, ll];
            let got = self.infile.read(itemlen - 2);
            if got.len() != itemlen - 2 {
                self.warn(format!(
                    "Error reading from file: got=x{:04X}(={}) != itemlen-2=x{:04X}(={})",
                    got.len() as u32,
                    got.len() as i32,
                    (itemlen - 2) as u32,
                    (itemlen - 2) as i32
                ));
                return false;
            }
            data.extend_from_slice(&got);
            let data: Rc<[u8]> = Rc::from(data);
            self.sections.push(data.clone());
            match marker {
                // exif_read_data never asks to read the whole image
                // (read_all), so this stops before the compressed data.
                M_SOS => return true,
                M_EOI => {
                    self.warn("No image in jpeg!");
                    return self.sections_found & !FOUND_COMPUTED != 0;
                }
                M_COM => {
                    let n = itemlen - 2;
                    self.add_tag(
                        SECTION_COMMENT,
                        b"Comment",
                        TAG_COMPUTED_VALUE,
                        FMT_STRING,
                        n,
                        Some((&data[..], 2)),
                        n,
                    );
                }
                M_EXIF => {
                    if self.sections_found & FOUND_IFD0 == 0 {
                        self.process_app1(&data, itemlen, fpos);
                    }
                }
                M_APP12 => self.process_app12(&data, itemlen),
                m if is_sof(m) => {
                    if itemlen - 2 < 6 {
                        return false;
                    }
                    let (h, w) = sof_size(&data, 0);
                    self.width = w as i32;
                    self.height = h as i32;
                    self.is_color = i32::from(data[7] == 3);
                }
                _ => {}
            }
            last_marker = i32::from(marker);
        }
    }

    /// exif_ifd_make_value: a directory entry's value bytes.
    fn ifd_make_value(d: &InfoData, mi: bool) -> Vec<u8> {
        let byte_count = BYTES_PER_FORMAT[d.format as usize] * d.length as usize;
        let mut out = vec![0u8; byte_count.max(4)];
        if d.length == 0 {
            return out;
        }
        match (&d.value, d.format) {
            (Payload::Str(s), _) => {
                // A one-byte BYTE/SBYTE value: php writes the union's
                // integer, which aliases the string pointer; rhp writes
                // the byte.
                let n = byte_count.min(s.len());
                out[..n].copy_from_slice(&s[..n]);
            }
            (Payload::Nums(nums), f) => {
                let mut p = 0;
                for n in nums.iter().take(d.length as usize) {
                    match f {
                        FMT_USHORT => {
                            set16u(&mut out[p..], n.u(), mi);
                            p += 2;
                        }
                        FMT_ULONG => {
                            set32u(&mut out[p..], u64::from(n.u()), mi);
                            p += 4;
                        }
                        FMT_SSHORT => {
                            set16u(&mut out[p..], n.i() as u32, mi);
                            p += 2;
                        }
                        FMT_SLONG => {
                            set32u(&mut out[p..], n.i() as i64 as u64, mi);
                            p += 4;
                        }
                        FMT_URATIONAL | FMT_SRATIONAL => {
                            let (a, b) = n.num_den();
                            set32u(&mut out[p..], u64::from(a), mi);
                            set32u(&mut out[p + 4..], u64::from(b), mi);
                            p += 8;
                        }
                        FMT_SINGLE => {
                            out[p..p + 4].copy_from_slice(&n.f().to_le_bytes());
                            p += 4;
                        }
                        FMT_DOUBLE => {
                            out[p..p + 8].copy_from_slice(&n.d().to_le_bytes());
                            p += 8;
                        }
                        _ => {}
                    }
                }
            }
            (Payload::None, _) => {}
        }
        out
    }

    /// exif_thumbnail_build: a TIFF thumbnail gets a TIFF header and IFD
    /// of its own.
    fn thumbnail_build(&mut self) {
        if !self.read_thumbnail || self.thumbnail.offset == 0 || self.thumbnail.size == 0 {
            return;
        }
        if rhp_values::mem::has_oom() {
            return;
        }
        if !matches!(
            self.thumbnail.filetype,
            IMAGE_FILETYPE_TIFF_II | IMAGE_FILETYPE_TIFF_MM
        ) {
            return;
        }
        let mi = self.mi();
        let list = &self.info_list[SECTION_THUMBNAIL];
        let mut new_size = 8 + 2 + list.len() * 12 + 4;
        let mut new_value = new_size;
        for d in list {
            let bc = BYTES_PER_FORMAT[d.format as usize] * d.length as usize;
            if bc > 4 {
                new_size += bc;
            }
        }
        let new_move = new_size;
        let old = self.thumbnail.data.take().unwrap_or_default();
        let mut buf = vec![0u8; new_size + self.thumbnail.size];
        let n = old.len().min(self.thumbnail.size);
        buf[new_move..new_move + n].copy_from_slice(&old[..n]);
        self.thumbnail.size += new_size;
        buf[..8].copy_from_slice(if mi {
            b"MM\x00\x2a\x00\x00\x00\x08"
        } else {
            b"II\x2a\x00\x08\x00\x00\x00"
        });
        set16u(&mut buf[8..], list.len() as u32, mi);
        let mut p = 10;
        for d in list {
            let bc = BYTES_PER_FORMAT[d.format as usize] * d.length as usize;
            let tag = u32::from(d.tag);
            set16u(&mut buf[p..], tag, mi);
            if tag == TAG_STRIP_OFFSETS || tag == TAG_JPEG_INTERCHANGE_FORMAT {
                set16u(&mut buf[p + 2..], u32::from(FMT_ULONG), mi);
                set32u(&mut buf[p + 4..], 1, mi);
                set32u(&mut buf[p + 8..], new_move as u64, mi);
            } else {
                set16u(&mut buf[p + 2..], u32::from(d.format), mi);
                set32u(&mut buf[p + 4..], u64::from(d.length), mi);
                let v = Self::ifd_make_value(d, mi);
                if bc <= 4 {
                    buf[p + 8..p + 12].copy_from_slice(&v[..4]);
                } else {
                    set32u(&mut buf[p + 8..], new_value as u64, mi);
                    buf[new_value..new_value + bc].copy_from_slice(&v[..bc]);
                    new_value += bc;
                }
            }
            p += 12;
        }
        // Next IFD pointer: none.
        buf[p..p + 4].fill(0);
        self.thumbnail.data = Some(buf);
    }

    /// exif_thumbnail_in_file.
    fn thumbnail_in_file(&mut self) -> bool {
        if self.thumbnail.size > self.file_size
            || self.thumbnail.offset > self.file_size - self.thumbnail.size
        {
            self.warn(ERROR_THUMBEOF);
            return false;
        }
        true
    }

    /// exif_thumbnail_extract.
    fn thumbnail_extract(&mut self, info: &OffsetInfo) {
        if self.thumbnail.data.is_some() {
            self.warn("Multiple possible thumbnails");
            return;
        }
        if !self.read_thumbnail {
            return;
        }
        // Exif 2.1: at most 64K.
        if self.thumbnail.size >= 65536 || self.thumbnail.size == 0 || self.thumbnail.offset == 0 {
            self.warn("Illegal thumbnail size/offset");
            return;
        }
        let Some(pos) = info.try_get(self.thumbnail.offset as u64, self.thumbnail.size as u64)
        else {
            self.warn(ERROR_THUMBEOF);
            return;
        };
        self.thumbnail.data = Some(info.mem[pos..pos + self.thumbnail.size].to_vec());
        self.thumbnail_build();
    }

    /// Reads a TIFF thumbnail at its offset.
    fn read_tiff_thumbnail(&mut self) -> bool {
        self.infile.seek(self.thumbnail.offset as i64);
        let data = self.infile.read(self.thumbnail.size);
        if data.len() != self.thumbnail.size {
            self.warn(ERROR_THUMBEOF);
            self.thumbnail.data = None;
        } else {
            self.thumbnail.data = Some(data);
            self.thumbnail_build();
        }
        true
    }

    /// exif_process_IFD_in_TIFF.
    fn process_ifd_in_tiff(&mut self, dir_offset: usize, section: usize) -> bool {
        let count = self.ifd_count;
        self.ifd_count += 1;
        if count > MAX_IFD_TAGS
            || self.ifd_nesting_level > MAX_IFD_NESTING_LEVEL
            || rhp_values::mem::has_oom()
        {
            return false;
        }
        self.ifd_nesting_level += 1;
        let r = self.process_ifd_in_tiff_impl(dir_offset, section);
        self.ifd_nesting_level -= 1;
        r
    }

    fn process_ifd_in_tiff_impl(&mut self, dir_offset: usize, section: usize) -> bool {
        let table = tag_table(section);
        let fs = self.file_size;
        if !(fs >= 2 && fs - 2 >= dir_offset) {
            self.warn(format!(
                "Error in TIFF: filesize(x{:04X}) less than start of IFD dir(x{:04X})",
                fs as u32,
                (dir_offset + 2) as u32
            ));
            return false;
        }
        self.infile.seek(dir_offset as i64);
        let mut data = self.infile.read(2);
        if data.len() != 2 {
            return false;
        }
        let mi = self.mi();
        let num_entries = get16u(&data, 0, mi) as usize;
        let dir_size = 2 + 12 * num_entries + 4;
        if !(fs >= dir_size && fs - dir_size >= dir_offset) {
            self.warn(format!(
                "Error in TIFF: filesize(x{:04X}) less than size of IFD dir(x{:04X})",
                fs as u32,
                (dir_offset + dir_size) as u32
            ));
            return false;
        }
        let rest = self.infile.read(dir_size - 2);
        if rest.len() != dir_size - 2 {
            return false;
        }
        data.extend_from_slice(&rest);
        let next_offset = get32u(&data, dir_size - 4, mi) as usize;
        // How far the IFD's values reach.
        let mut ifd_size = dir_size;
        let mut entry_value: usize = 0;
        for i in 0..num_entries {
            let e = 2 + i * 12;
            let entry_tag = get16u(&data, e, mi);
            let mut entry_type = get16u(&data, e + 2, mi);
            if entry_type > NUM_FORMATS {
                self.error(
                    E_NOTICE,
                    format!(
                        "Read from TIFF: tag(0x{entry_tag:04X},{:>12}): Illegal format code 0x{entry_type:04X}, switching to BYTE",
                        tagname_debug(entry_tag, table)
                    ),
                );
                entry_type = u32::from(FMT_BYTE);
            }
            // unsigned arithmetic, as in C.
            let entry_length = get32u(&data, e + 4, mi)
                .wrapping_mul(BYTES_PER_FORMAT[entry_type as usize] as u32)
                as usize;
            if entry_length <= 4 {
                match entry_type as u16 {
                    FMT_USHORT => entry_value = get16u(&data, e + 8, mi) as usize,
                    FMT_SSHORT => entry_value = get16s(&data, e + 8, mi) as isize as usize,
                    FMT_ULONG | FMT_SLONG => entry_value = get32u(&data, e + 8, mi) as usize,
                    _ => {}
                }
                match entry_tag {
                    TAG_NEW_SUBFILE_IMAGEWIDTH | TAG_COMP_IMAGE_WIDTH => {
                        self.width = entry_value as i32
                    }
                    TAG_IMAGEHEIGHT | TAG_COMP_IMAGE_HEIGHT => self.height = entry_value as i32,
                    TAG_PHOTOMETRIC_INTERPRETATION => match entry_value {
                        0 | 1 | 4 => self.is_color = 0,
                        2 | 3 | 5 | 6 | 8 => self.is_color = 1,
                        _ => {}
                    },
                    _ => {}
                }
            } else {
                let entry_offset = get32u(&data, e + 8, mi) as usize;
                // Grow the cache only for values right after it.
                if entry_offset + entry_length > dir_offset + ifd_size
                    && entry_offset == dir_offset + ifd_size
                {
                    ifd_size = entry_offset + entry_length - dir_offset;
                }
            }
        }
        if !(fs >= data.len() && fs - data.len() >= dir_offset) {
            self.warn(format!(
                "Error in TIFF: filesize(x{:04X}) less than size of IFD(x{:04X})",
                fs as u32,
                (dir_offset + data.len()) as u32
            ));
            return false;
        }
        if ifd_size > dir_size {
            if fs < ifd_size || dir_offset > fs - ifd_size {
                self.warn(format!(
                    "Error in TIFF: filesize(x{:04X}) less than size of IFD(x{:04X} + x{:04X})",
                    fs as u32, dir_offset as u32, ifd_size as u32
                ));
                return false;
            }
            // The values outside the directory (a short read leaves zeros).
            let more = self.infile.read(ifd_size - dir_size);
            data.extend_from_slice(&more);
            data.resize(ifd_size, 0);
        }
        let mem: Rc<[u8]> = Rc::from(data);
        for i in 0..num_entries {
            let e = 2 + i * 12;
            let mi = self.mi();
            let entry_tag = get16u(&mem, e, mi);
            if matches!(
                entry_tag,
                TAG_EXIF_IFD_POINTER | TAG_INTEROP_IFD_POINTER | TAG_GPS_IFD_POINTER | TAG_SUB_IFD
            ) {
                let sub = match entry_tag {
                    TAG_EXIF_IFD_POINTER => {
                        self.sections_found |= FOUND_EXIF;
                        SECTION_EXIF
                    }
                    TAG_GPS_IFD_POINTER => {
                        self.sections_found |= FOUND_GPS;
                        SECTION_GPS
                    }
                    TAG_INTEROP_IFD_POINTER => {
                        self.sections_found |= FOUND_INTEROP;
                        SECTION_INTEROP
                    }
                    _ => {
                        self.sections_found |= FOUND_THUMBNAIL;
                        SECTION_THUMBNAIL
                    }
                };
                let entry_offset = get32u(&mem, e + 8, mi) as usize;
                self.process_ifd_in_tiff(entry_offset, sub);
                if section != SECTION_THUMBNAIL
                    && entry_tag == TAG_SUB_IFD
                    && self.thumbnail.filetype != IMAGE_FILETYPE_UNKNOWN
                    && self.thumbnail.size != 0
                    && self.thumbnail.offset != 0
                    && self.read_thumbnail
                    && self.thumbnail.data.is_none()
                    && self.thumbnail_in_file()
                {
                    self.read_tiff_thumbnail();
                }
            } else {
                let info = OffsetInfo::new(mem.clone(), -(dir_offset as i64), 0, ifd_size);
                let entry = Ptr {
                    mem: mem.clone(),
                    pos: e,
                };
                if !self.process_ifd_tag(&entry, &info, 0, section, false, table) {
                    return false;
                }
            }
        }
        // After a SUB_IFD thumbnail the next IFD is another image.
        if next_offset != 0 && section != SECTION_THUMBNAIL {
            self.process_ifd_in_tiff(next_offset, SECTION_THUMBNAIL);
            if self.thumbnail.data.is_none()
                && self.thumbnail.offset != 0
                && self.thumbnail.size != 0
                && self.read_thumbnail
                && self.thumbnail_in_file()
            {
                self.read_tiff_thumbnail();
            }
        }
        true
    }

    /// exif_scan_HEIF_header.
    fn scan_heif_header(&mut self, buf: &mut [u8; 16]) -> bool {
        let fs = self.file_size as u64;
        let mut offset = u64::from(get32u(buf, 0, true));
        while fs.wrapping_sub(16) > offset {
            if !self.infile.seek(offset as i64) {
                break;
            }
            let got = self.infile.read(16);
            if got.len() != 16 {
                break;
            }
            buf.copy_from_slice(&got);
            let (hsize, bsize, btype) = parse_box(buf, 0);
            if bsize < hsize as u64 {
                break;
            }
            if btype == u32::from_be_bytes(*b"meta") {
                let mut limit = bsize - hsize as u64;
                if limit < 36 || limit > fs {
                    break;
                }
                let remain = 16 - hsize;
                let mut data = buf[hsize..].to_vec();
                let more = self.infile.read(limit as usize - remain);
                let mut pos = (0u32, 0u32);
                if more.len() == limit as usize - remain {
                    data.extend_from_slice(&more);
                    pos = parse_meta(&data);
                }
                let (p_off, p_size) = (u64::from(pos.0), u64::from(pos.1));
                if p_size >= 2
                    && p_size < fs
                    && fs - p_size >= p_off
                    && self.infile.seek((p_off + 2) as i64)
                {
                    limit = p_size - 2;
                    let d = self.infile.read(limit as usize);
                    if d.len() as u64 == limit {
                        let mem: Rc<[u8]> = Rc::from(d);
                        self.process_app1(&mem, limit as usize, (p_off + 2) as usize);
                        return true;
                    }
                }
                break;
            }
            match offset.checked_add(bsize) {
                Some(n) => offset = n,
                None => break,
            }
        }
        false
    }

    /// exif_scan_FILE_header.
    fn scan_file_header(&mut self) -> bool {
        self.file_type = IMAGE_FILETYPE_UNKNOWN;
        if self.file_size < 2 {
            self.warn(format!("File too small ({})", self.file_size as i32));
            return false;
        }
        self.infile.seek(0);
        let mut header = [0u8; 16];
        let got = self.infile.read(2);
        if got.len() != 2 {
            return false;
        }
        header[..2].copy_from_slice(&got);
        if header[0] == 0xFF && header[1] == M_SOI {
            self.file_type = IMAGE_FILETYPE_JPEG;
            if self.scan_jpeg_header() {
                return true;
            }
            self.warn("Invalid JPEG file");
            return false;
        }
        if self.file_size < 8 {
            return false;
        }
        let got = self.infile.read(6);
        if got.len() != 6 {
            return false;
        }
        header[2..8].copy_from_slice(&got);
        if header[..4] == *b"II\x2A\x00" || header[..4] == *b"MM\x00\x2A" {
            let motorola = header[0] == b'M';
            self.file_type = if motorola {
                IMAGE_FILETYPE_TIFF_MM
            } else {
                IMAGE_FILETYPE_TIFF_II
            };
            self.motorola_intel = i32::from(motorola);
            self.sections_found |= FOUND_IFD0;
            let off = get32u(&header, 4, motorola) as usize;
            if self.process_ifd_in_tiff(off, SECTION_IFD0) {
                return true;
            }
            self.warn("Invalid TIFF file");
            return false;
        }
        if self.file_size > 16 && header[4..8] == *b"ftyp" {
            let got = self.infile.read(4);
            if got.len() == 4 {
                header[8..12].copy_from_slice(&got);
                if matches!(&header[8..12], b"heic" | b"heix" | b"mif1") {
                    if self.scan_heif_header(&mut header) {
                        self.file_type = IMAGE_FILETYPE_HEIF;
                        return true;
                    }
                    self.warn("Invalid HEIF file");
                    return false;
                }
            }
        }
        self.warn("File not supported");
        false
    }
}

/// strncmp(a, b, n) == 0, with C strings' NUL ends.
fn strncmp_eq(a: &[u8], b: &[u8], n: usize) -> bool {
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return false;
        }
        if x == 0 {
            return true;
        }
    }
    true
}

/// exif_isobmff_parse_box: (header size, box size, type).
fn parse_box(buf: &[u8], pos: usize) -> (usize, u64, u32) {
    let size = get32u(buf, pos, true);
    let t = get32u(buf, pos + 4, true);
    if size != 1 {
        return (8, u64::from(size), t);
    }
    (16, get64u(buf, pos + 8, true), t)
}

/// exif_isobmff_parse_meta: the Exif item's (offset, size).
#[allow(unused_assignments)]
fn parse_meta(data: &[u8]) -> (u32, u32) {
    let end = data.len();
    let mut pos = (0u32, 0u32);
    let mut exif_id: i64 = -1;
    let mut box_offset = 4usize;
    while (box_offset as i64) < end as i64 - 16 {
        let (hsize, bsize, btype) = parse_box(data, box_offset);
        if bsize < hsize as u64 {
            return pos;
        }
        let mut p = box_offset;
        let mut remain = end - p;
        macro_rules! advance {
            ($n:expr) => {{
                let n = $n as usize;
                if remain < n {
                    return pos;
                }
                remain -= n;
                p += n;
            }};
        }
        if btype == u32::from_be_bytes(*b"iinf") {
            advance!(hsize + 4);
            let version = data[p - 4];
            let item_count = if version < 2 {
                advance!(2);
                i64::from(get16u(data, p - 2, true))
            } else {
                advance!(4);
                i64::from(get32u(data, p - 4, true) as i32)
            };
            let mut i = 0;
            while i < item_count && (p as i64) < end as i64 - 20 {
                let (ih, isize, _) = parse_box(data, p);
                if isize < ih as u64 {
                    return pos;
                }
                if remain < ih + 12 {
                    return pos;
                }
                if data[p + ih + 8..p + ih + 12] == *b"Exif" {
                    exif_id = i64::from(get16u(data, p + ih + 4, true));
                    break;
                }
                if (remain as u64) < isize {
                    return pos;
                }
                remain -= isize as usize;
                p += isize as usize;
                i += 1;
            }
            if exif_id < 0 {
                break;
            }
        } else if btype == u32::from_be_bytes(*b"iloc") {
            advance!(hsize + 6);
            let version = data[p - 6];
            let item_count = if version < 2 {
                advance!(2);
                i64::from(get16u(data, p - 2, true))
            } else {
                advance!(4);
                i64::from(get32u(data, p - 4, true) as i32)
            };
            let mut i = 0;
            while i < item_count && (p as i64) < end as i64 - 16 {
                if i64::from(get16u(data, p, true)) == exif_id {
                    pos = (get32u(data, p + 8, true), get32u(data, p + 12, true));
                    break;
                }
                i += 1;
                p += 16;
            }
            break;
        }
        if (end as i64 - 16 - box_offset as i64) as u64 <= bsize {
            break;
        }
        box_offset += bsize as usize;
    }
    pos
}

// ---------------------------------------------------------------------------
// reading a file
// ---------------------------------------------------------------------------

/// exif_read_from_impl over an open stream.
fn read_from_impl(
    host: &mut dyn Host,
    res: Value,
    orig_path: Option<Vec<u8>>,
    read_thumbnail: bool,
) -> (bool, ImageInfoOut) {
    let is_stdio = matches!(
        host.stream_kind(&res),
        Some(crate::StreamKind::File | crate::StreamKind::Stdio | crate::StreamKind::Pipe)
    );
    let mut ii = ImageInfo::new(Infile { host, res });
    if is_stdio {
        if let Some(path) = orig_path.filter(|p| !p.is_empty()) {
            let local = crate::zend::virtual_cwd::expand_local(&path);
            use std::os::unix::ffi::OsStrExt;
            if let Ok(md) = std::fs::metadata(std::ffi::OsStr::from_bytes(&local)) {
                if !md.is_file() {
                    ii.warn("Not a file");
                    return (false, ImageInfoOut::from(ii));
                }
                use std::os::unix::fs::MetadataExt;
                ii.file_name = Some(crate::ext::standard::string::php_basename(&path).to_vec());
                ii.file_date_time = md.mtime();
                ii.file_size = md.size() as usize;
            }
        }
    } else {
        ii.infile.seek_end();
        ii.file_size = ii.infile.tell().max(0) as usize;
        ii.infile.seek(0);
    }
    ii.read_thumbnail = read_thumbnail;
    let ret = ii.scan_file_header();
    (ret, ImageInfoOut::from(ii))
}

/// What the PHP functions use once the file is read (the stream's host
/// borrow ended).
struct ImageInfoOut {
    file_name: Option<Vec<u8>>,
    file_date_time: i64,
    file_size: usize,
    file_type: i64,
    height: i32,
    width: i32,
    is_color: i32,
    aperture_fnumber: f32,
    exposure_time: f32,
    ccd_width: f32,
    distance: f32,
    motorola_intel: i32,
    user_comment: Option<Vec<u8>>,
    user_comment_length: i32,
    user_comment_encoding: Option<Vec<u8>>,
    copyright: Option<Vec<u8>>,
    copyright_photographer: Option<Vec<u8>>,
    copyright_editor: Option<Vec<u8>>,
    xp_fields: Vec<(u32, Vec<u8>)>,
    thumbnail: Thumbnail,
    sections_found: i32,
    info_list: [Vec<InfoData>; SECTION_COUNT],
    diags: Vec<(i64, Vec<u8>, Vec<u8>)>,
    num_errors: i32,
}

impl From<ImageInfo<'_>> for ImageInfoOut {
    fn from(i: ImageInfo<'_>) -> ImageInfoOut {
        ImageInfoOut {
            file_name: i.file_name,
            file_date_time: i.file_date_time,
            file_size: i.file_size,
            file_type: i.file_type,
            height: i.height,
            width: i.width,
            is_color: i.is_color,
            aperture_fnumber: i.aperture_fnumber,
            exposure_time: i.exposure_time,
            ccd_width: i.ccd_width,
            distance: i.distance,
            motorola_intel: i.motorola_intel,
            user_comment: i.user_comment,
            user_comment_length: i.user_comment_length,
            user_comment_encoding: i.user_comment_encoding,
            copyright: i.copyright,
            copyright_photographer: i.copyright_photographer,
            copyright_editor: i.copyright_editor,
            xp_fields: i.xp_fields,
            thumbnail: i.thumbnail,
            sections_found: i.sections_found,
            info_list: i.info_list,
            diags: i.diags,
            num_errors: i.num_errors,
        }
    }
}

/// Opens `path` as php_stream_open_wrapper does without REPORT_ERRORS:
/// a failure says nothing (open_basedir still warns).
fn open_quiet(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<Option<Value>, Thrown> {
    use crate::main::streams::{locate_url_wrapper, Located};
    let located = locate_url_wrapper(path, |s| crate::diag::wrapper_registered(host, s));
    let target = match &located {
        Located::Plain(_) => path.to_vec(),
        Located::Wrapper(p) => p.clone(),
        Located::Unknown(_) => crate::zend::virtual_cwd::expand_local(path),
        Located::RemoteHost => return Ok(None),
    };
    if matches!(located, Located::Plain(_) | Located::Unknown(_))
        && crate::main::fopen_wrappers::active()
    {
        let local = match &located {
            Located::Plain(_) => crate::main::streams::file_path(path).to_vec(),
            _ => path.to_vec(),
        };
        if !crate::main::fopen_wrappers::check_open(host, fname, &local, path)? {
            return Ok(None);
        }
    }
    crate::main::streams::clear_errno();
    let r = host.stream_open_ctx(&target, b"rb", &Value::Null);
    if r.is_none() {
        crate::main::streams::plain_wrapper::take_diags();
    }
    Ok(r)
}

/// exif_read_from_stream / exif_read_from_file for the first argument.
fn read_file(
    host: &mut dyn Host,
    fname: &str,
    args: &[Value],
    read_thumbnail: bool,
) -> Result<(bool, ImageInfoOut), Thrown> {
    let arg = args.first().map(deref).unwrap_or(Value::Null);
    if let Value::Resource(_) = arg {
        if host.stream_kind(&arg).is_none() {
            return Err(Thrown::type_error(format!(
                "{fname}(): supplied resource is not a valid stream resource"
            )));
        }
        let uri = host.stream_meta(&arg).map(|m| m.uri);
        // exif_read_from_stream: from the start, then back.
        let old_pos = host.stream_tell(&arg).unwrap_or(0);
        if old_pos != 0 {
            host.stream_seek(&arg, 0, libc::SEEK_SET);
        }
        let out = read_from_impl(host, arg.clone(), uri, read_thumbnail);
        if old_pos != 0 {
            host.stream_seek(&arg, old_pos, libc::SEEK_SET);
        }
        return Ok(out);
    }
    let path = zpp::zval_get_string(host, &arg)?;
    let pname = rhp_bytecode::signatures::function(fname.as_bytes())
        .and_then(|f| f.params.first())
        .map_or("file", |p| p.name);
    if path.is_empty() {
        return Err(zpp::value_error(fname, 1, pname, "must not be empty"));
    }
    if path.contains(&0) {
        return Err(zpp::value_error(
            fname,
            1,
            pname,
            "must not contain any null bytes",
        ));
    }
    let Some(res) = open_quiet(host, fname, &path)? else {
        let out = ImageInfoOut {
            diags: vec![(E_WARNING, Vec::new(), b"Unable to open file".to_vec())],
            ..ImageInfoOut::empty()
        };
        return Ok((false, out));
    };
    let out = read_from_impl(host, res.clone(), Some(path), read_thumbnail);
    host.stream_close(&res);
    Ok(out)
}

impl ImageInfoOut {
    fn empty() -> ImageInfoOut {
        ImageInfoOut {
            file_name: None,
            file_date_time: 0,
            file_size: 0,
            file_type: 0,
            height: 0,
            width: 0,
            is_color: 0,
            aperture_fnumber: 0.0,
            exposure_time: 0.0,
            ccd_width: 0.0,
            distance: 0.0,
            motorola_intel: -1,
            user_comment: None,
            user_comment_length: 0,
            user_comment_encoding: None,
            copyright: None,
            copyright_photographer: None,
            copyright_editor: None,
            xp_fields: Vec::new(),
            thumbnail: Thumbnail::default(),
            sections_found: 0,
            info_list: Default::default(),
            diags: Vec::new(),
            num_errors: 0,
        }
    }

    /// The diagnostics, as php_verror raises them from `fname`.
    fn emit(&mut self, host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
        for (level, file, msg) in std::mem::take(&mut self.diags) {
            let mut m = fname.as_bytes().to_vec();
            m.push(b'(');
            m.extend_from_slice(&file);
            m.extend_from_slice(b"): ");
            m.extend_from_slice(&msg);
            host.emit_diagnostic(level, &m)?;
        }
        Ok(())
    }

    /// The image_info list adders, for the sections exif_read_data builds.
    fn add_int(&mut self, section: usize, name: &str, value: i32) {
        self.info_list[section].push(InfoData {
            tag: TAG_NONE,
            format: FMT_SLONG,
            length: 1,
            name: name.as_bytes().to_vec(),
            value: Payload::Nums(vec![Num(u64::from(value as u32))]),
        });
        self.sections_found |= 1 << section;
    }

    fn add_str(&mut self, section: usize, name: &[u8], value: Option<&[u8]>) {
        if let Some(v) = value {
            self.info_list[section].push(InfoData {
                tag: TAG_NONE,
                format: FMT_STRING,
                length: 1,
                name: name.to_vec(),
                value: Payload::Str(cstr(v).to_vec()),
            });
            self.sections_found |= 1 << section;
        }
    }

    fn add_bytes(&mut self, section: usize, name: &str, value: Vec<u8>, length: u32) {
        self.info_list[section].push(InfoData {
            tag: TAG_NONE,
            format: FMT_UNDEFINED,
            length,
            name: name.as_bytes().to_vec(),
            value: Payload::Str(value),
        });
        self.sections_found |= 1 << section;
    }

    /// exif_scan_thumbnail on the read thumbnail (it only warns).
    fn scan_thumbnail(&mut self) -> bool {
        let size = self.thumbnail.size;
        let Some(data) = self.thumbnail.data.clone() else {
            return false;
        };
        if size < 4 {
            return false;
        }
        if !data.starts_with(b"\xFF\xD8\xFF") {
            if self.thumbnail.width == 0 && self.thumbnail.height == 0 {
                self.warn_late("Thumbnail is not a JPEG image");
            }
            return false;
        }
        match walk_thumbnail(&data, size) {
            Walk::Sof(h, w) => {
                self.thumbnail.height = h;
                self.thumbnail.width = w;
                true
            }
            Walk::SosOrEoi => {
                self.warn_late("Could not compute size of thumbnail");
                false
            }
            Walk::Fail => false,
        }
    }

    /// exif_error_docref after the read.
    fn warn_late(&mut self, msg: &str) {
        self.num_errors += 1;
        if self.num_errors > EXIF_MAX_ERRORS {
            if self.num_errors == EXIF_MAX_ERRORS + 1 {
                self.diags.push((
                    E_WARNING,
                    Vec::new(),
                    b"Further exif parsing errors have been suppressed".to_vec(),
                ));
            }
            return;
        }
        let file = self.file_name.clone().unwrap_or_default();
        self.diags.push((E_WARNING, file, msg.as_bytes().to_vec()));
    }
}

enum Walk {
    Sof(usize, usize),
    SosOrEoi,
    Fail,
}

/// exif_scan_thumbnail's loop over a JPEG thumbnail.
fn walk_thumbnail(data: &[u8], size: usize) -> Walk {
    let at = |i: usize| data.get(i).copied().unwrap_or(0);
    let mut length = 2usize;
    let mut pos = 0usize;
    loop {
        pos += length;
        if pos >= size {
            return Walk::Fail;
        }
        let mut c = at(pos);
        pos += 1;
        if pos >= size || c != 0xFF {
            return Walk::Fail;
        }
        let mut n = 8;
        loop {
            c = at(pos);
            pos += 1;
            let more = c == 0xFF && n != 0;
            n -= 1;
            if !more {
                break;
            }
            if pos + 3 >= size {
                return Walk::Fail;
            }
        }
        if c == 0xFF {
            return Walk::Fail;
        }
        if pos >= size {
            return Walk::Fail;
        }
        length = jpg_get16(data, pos) as usize;
        if length > size || pos >= size - length {
            return Walk::Fail;
        }
        if is_sof(c) {
            if length < 8 || size - 8 < pos {
                return Walk::Fail;
            }
            let (h, w) = sof_size(data, pos);
            return Walk::Sof(h, w);
        }
        if c == M_SOS || c == M_EOI {
            return Walk::SosOrEoi;
        }
    }
}

/// `%0.Nf`-style formatting through php's conv_fp.
fn fmt_f(v: f64, precision: usize) -> String {
    let (digits, neg) = rhp_values::fmt::conv_fp(b'F', v, precision, false, b'.');
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&String::from_utf8_lossy(&digits));
    s
}

// ---------------------------------------------------------------------------
// PHP functions
// ---------------------------------------------------------------------------

fn pname(fname: &str, i: usize) -> &'static str {
    rhp_bytecode::signatures::function(fname.as_bytes())
        .and_then(|f| f.params.get(i))
        .map_or("", |p| p.name)
}

/// `exif_read_data($file, ?string $required_sections = null, bool $as_arrays = false, bool $read_thumbnail = false): array|false`.
fn fn_exif_read_data(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "exif_read_data";
    let sections = zpp::opt_str_arg(host, args, 1, f, pname(f, 1))?;
    let sub_arrays = zpp::bool_arg_or(host, args, 2, f, pname(f, 2), false)?;
    let read_thumbnail = zpp::bool_arg_or(host, args, 3, f, pname(f, 3), false)?;
    let mut needed = 0i32;
    if let Some(s) = sections {
        let mut list = b",".to_vec();
        list.extend(cstr(&s).iter().map(|&c| if c == b' ' { b',' } else { c }));
        list.push(b',');
        for i in 0..SECTION_COUNT {
            let probe = format!(",{},", section_name(i));
            if list.windows(probe.len()).any(|w| w == probe.as_bytes()) {
                needed |= 1 << i;
            }
        }
    }
    let (ret, mut ii) = read_file(host, f, args, read_thumbnail)?;
    // Over memory_limit: the fatal error comes at the VM's next check.
    if rhp_values::mem::has_oom() {
        return Ok(Value::Bool(false));
    }
    let sections_str = section_list(ii.sections_found);
    ii.sections_found |= FOUND_COMPUTED | FOUND_FILE;
    if !ret || (needed != 0 && needed & ii.sections_found == 0) {
        ii.emit(host, f)?;
        return Ok(Value::Bool(false));
    }
    let file_name = ii.file_name.clone();
    ii.add_str(SECTION_FILE, b"FileName", file_name.as_deref());
    ii.add_int(SECTION_FILE, "FileDateTime", ii.file_date_time as i32);
    ii.add_int(SECTION_FILE, "FileSize", ii.file_size as i32);
    ii.add_int(SECTION_FILE, "FileType", ii.file_type as i32);
    let mime = crate::ext::standard::image::mime_type(ii.file_type);
    ii.add_str(SECTION_FILE, b"MimeType", Some(mime.as_bytes()));
    ii.add_str(SECTION_FILE, b"SectionsFound", Some(&sections_str));

    if ii.width > 0 && ii.height > 0 {
        let html = format!("width=\"{}\" height=\"{}\"", ii.width, ii.height);
        ii.add_str(SECTION_COMPUTED, b"html", Some(html.as_bytes()));
        ii.add_int(SECTION_COMPUTED, "Height", ii.height);
        ii.add_int(SECTION_COMPUTED, "Width", ii.width);
    }
    ii.add_int(SECTION_COMPUTED, "IsColor", ii.is_color);
    if ii.motorola_intel != -1 {
        ii.add_int(SECTION_COMPUTED, "ByteOrderMotorola", ii.motorola_intel);
    }
    // FocalLength (and 35mmFocalLength) need ImageInfo.FocalLength, which
    // exif.c never sets.
    if ii.ccd_width != 0.0 {
        let s = format!("{}mm", ii.ccd_width as i32);
        ii.add_str(SECTION_COMPUTED, b"CCDWidth", Some(s.as_bytes()));
    }
    if ii.exposure_time > 0.0 {
        let recip = 0.5f32 + 1.0f32 / ii.exposure_time;
        let s = if f64::from(ii.exposure_time) <= 0.5 && recip < i32::MAX as f32 {
            format!(
                "{} s (1/{})",
                fmt_f(f64::from(ii.exposure_time), 3),
                recip as i32
            )
        } else {
            format!("{} s", fmt_f(f64::from(ii.exposure_time), 3))
        };
        ii.add_str(SECTION_COMPUTED, b"ExposureTime", Some(s.as_bytes()));
    }
    if ii.aperture_fnumber != 0.0 {
        let s = format!("f/{}", fmt_f(f64::from(ii.aperture_fnumber), 1));
        ii.add_str(SECTION_COMPUTED, b"ApertureFNumber", Some(s.as_bytes()));
    }
    if ii.distance != 0.0 {
        if ii.distance < 0.0 {
            ii.add_str(SECTION_COMPUTED, b"FocusDistance", Some(b"Infinite"));
        } else {
            let s = format!("{}m", fmt_f(f64::from(ii.distance), 2));
            ii.add_str(SECTION_COMPUTED, b"FocusDistance", Some(s.as_bytes()));
        }
    }
    if let Some(c) = ii.user_comment.clone() {
        // exif_iif_add_buffer copies UserCommentLength bytes.
        let len = ii.user_comment_length;
        let mut v = c;
        v.resize(len.max(0) as usize, 0);
        ii.add_bytes(SECTION_COMPUTED, "UserComment", v, len as u32);
        if let Some(e) = ii.user_comment_encoding.clone().filter(|e| !e.is_empty()) {
            ii.add_str(SECTION_COMPUTED, b"UserCommentEncoding", Some(&e));
        }
    }
    let (c, cp, ce) = (
        ii.copyright.clone(),
        ii.copyright_photographer.clone(),
        ii.copyright_editor.clone(),
    );
    ii.add_str(SECTION_COMPUTED, b"Copyright", c.as_deref());
    ii.add_str(SECTION_COMPUTED, b"Copyright.Photographer", cp.as_deref());
    ii.add_str(SECTION_COMPUTED, b"Copyright.Editor", ce.as_deref());
    for (tag, v) in std::mem::take(&mut ii.xp_fields) {
        let name = tagname_debug(tag, tag_table(SECTION_WINXP));
        ii.add_str(SECTION_WINXP, name.as_bytes(), Some(&v));
    }
    if ii.thumbnail.size != 0 {
        if read_thumbnail {
            let size = ii.thumbnail.size;
            let (v, len) = match &ii.thumbnail.data {
                Some(d) => {
                    let mut v = d.clone();
                    v.resize(size, 0);
                    (v, size as u32)
                }
                None => (Vec::new(), 0),
            };
            ii.info_list[SECTION_THUMBNAIL].push(InfoData {
                tag: TAG_NONE,
                format: FMT_UNDEFINED,
                length: len,
                name: b"THUMBNAIL".to_vec(),
                value: Payload::Str(v),
            });
            ii.sections_found |= FOUND_THUMBNAIL;
        }
        if ii.thumbnail.width == 0 || ii.thumbnail.height == 0 {
            ii.scan_thumbnail();
        }
        ii.add_int(
            SECTION_COMPUTED,
            "Thumbnail.FileType",
            ii.thumbnail.filetype as i32,
        );
        let mime = crate::ext::standard::image::mime_type(ii.thumbnail.filetype);
        ii.add_str(
            SECTION_COMPUTED,
            b"Thumbnail.MimeType",
            Some(mime.as_bytes()),
        );
    }
    if ii.thumbnail.width != 0 && ii.thumbnail.height != 0 {
        ii.add_int(
            SECTION_COMPUTED,
            "Thumbnail.Height",
            ii.thumbnail.height as i32,
        );
        ii.add_int(
            SECTION_COMPUTED,
            "Thumbnail.Width",
            ii.thumbnail.width as i32,
        );
    }
    ii.emit(host, f)?;
    let mut out = PhpArray::new();
    for (section, sub) in [
        (SECTION_FILE, sub_arrays),
        (SECTION_COMPUTED, true),
        (SECTION_ANY_TAG, sub_arrays),
        (SECTION_IFD0, sub_arrays),
        (SECTION_THUMBNAIL, true),
        (SECTION_COMMENT, true),
        (SECTION_EXIF, sub_arrays),
        (SECTION_GPS, sub_arrays),
        (SECTION_INTEROP, sub_arrays),
        (SECTION_FPIX, sub_arrays),
        (SECTION_APP12, sub_arrays),
        (SECTION_WINXP, sub_arrays),
        (SECTION_MAKERNOTE, sub_arrays),
    ] {
        add_assoc_list(&mut out, sub, &ii.info_list[section], section);
    }
    Ok(Value::Array(out))
}

/// add_assoc_image_info.
fn add_assoc_list(out: &mut PhpArray, sub_array: bool, list: &[InfoData], section: usize) {
    if list.is_empty() {
        return;
    }
    let mut tmp = PhpArray::new();
    {
        let target: &mut PhpArray = if sub_array { &mut tmp } else { out };
        add_list_into(target, list, section);
    }
    if sub_array {
        out.insert(
            ArrayKey::from_bytes(section_name(section).as_bytes()),
            Value::Array(tmp),
        );
    }
}

fn add_list_into(target: &mut PhpArray, list: &[InfoData], section: usize) {
    let mut idx = 0i64;
    for d in list {
        let key = ArrayKey::from_bytes(&d.name);
        if d.length == 0 {
            target.insert(key, Value::Null);
            continue;
        }
        match d.format {
            FMT_STRING => {
                let s = match &d.value {
                    Payload::Str(s) => cstr(s).to_vec(),
                    _ => Vec::new(),
                };
                if section == SECTION_COMMENT {
                    target.insert(ArrayKey::Int(idx), Value::str(s));
                    idx += 1;
                } else {
                    target.insert(key, Value::str(s));
                }
            }
            FMT_URATIONAL | FMT_SRATIONAL | FMT_USHORT | FMT_SSHORT | FMT_SINGLE | FMT_DOUBLE
            | FMT_ULONG | FMT_SLONG => {
                let nums = match &d.value {
                    Payload::Nums(n) => n.as_slice(),
                    _ => &[],
                };
                let l = d.length as usize;
                let mut arr = PhpArray::new();
                for ap in 0..l {
                    let n = nums.get(ap).copied().unwrap_or_default();
                    let v = match d.format {
                        FMT_USHORT | FMT_ULONG => Value::Int(i64::from(n.u() as i32)),
                        FMT_URATIONAL => {
                            let (a, b) = n.num_den();
                            Value::str(format!("{a}/{b}"))
                        }
                        FMT_SSHORT | FMT_SLONG => Value::Int(i64::from(n.i())),
                        FMT_SRATIONAL => {
                            let (a, b) = n.num_den();
                            Value::str(format!("{}/{}", a as i32, b as i32))
                        }
                        FMT_SINGLE => Value::Float(f64::from(n.f())),
                        _ => Value::Float(n.d()),
                    };
                    if l == 1 {
                        target.insert(key.clone(), v);
                    } else {
                        arr.insert(ArrayKey::Int(ap as i64), v);
                    }
                }
                if l > 1 {
                    target.insert(key, Value::Array(arr));
                }
            }
            _ => {
                let s = match &d.value {
                    Payload::Str(s) => {
                        let mut v = s.clone();
                        v.resize(d.length as usize, 0);
                        v
                    }
                    _ => Vec::new(),
                };
                target.insert(key, Value::Str(PhpStr::from_bytes(s)));
            }
        }
    }
}

/// `exif_thumbnail($file, &$width = null, &$height = null, &$image_type = null): string|false`.
fn fn_exif_thumbnail(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "exif_thumbnail";
    let (ret, mut ii) = read_file(host, f, args, true)?;
    if !ret {
        ii.emit(host, f)?;
        return Ok(Value::Bool(false));
    }
    let Some(data) = ii.thumbnail.data.clone().filter(|_| ii.thumbnail.size != 0) else {
        ii.emit(host, f)?;
        return Ok(Value::Bool(false));
    };
    let mut data = data;
    data.resize(ii.thumbnail.size, 0);
    if (args.len() > 1 || args.len() > 2)
        && (ii.thumbnail.width == 0 || ii.thumbnail.height == 0)
        && !ii.scan_thumbnail()
    {
        ii.thumbnail.width = 0;
        ii.thumbnail.height = 0;
    }
    ii.emit(host, f)?;
    if args.len() > 1 {
        zpp::try_assign_ref(host, args, 1, Value::Int(ii.thumbnail.width as i64))?;
    }
    if args.len() > 2 {
        zpp::try_assign_ref(host, args, 2, Value::Int(ii.thumbnail.height as i64))?;
    }
    if args.len() > 3 {
        zpp::try_assign_ref(host, args, 3, Value::Int(ii.thumbnail.filetype))?;
    }
    Ok(Value::str(data))
}

/// `exif_tagname(int $index): string|false`.
fn fn_exif_tagname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "exif_tagname";
    let tag = zpp::long_arg(host, args, 0, f, pname(f, 0))?;
    match tagname(tag, tags::TAG_TABLE_IFD) {
        Some(name) if tag >= 0 => Ok(Value::str(name)),
        _ => Ok(Value::Bool(false)),
    }
}

/// `exif_imagetype(string $filename): int|false`.
fn fn_exif_imagetype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "exif_imagetype";
    let path = zpp::path_param(host, args, 0, f, pname(f, 0))?;
    // php_stream_open_wrapper refuses an empty path.
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    let Some(res) =
        crate::ext::standard::file::open_stream(host, f, &path, b"rb", &Value::Null, false)?
    else {
        return Ok(Value::Bool(false));
    };
    let data = host.stream_read(&res, -1).unwrap_or_default();
    host.stream_close(&res);
    let t = crate::ext::standard::image::getimagetype(host, f, &data, &path)?;
    Ok(if t == IMAGE_FILETYPE_UNKNOWN {
        Value::Bool(false)
    } else {
        Value::Int(t)
    })
}

/// OnUpdateEncode / OnUpdateDecode: an encoding list mbstring refuses
/// keeps the old value; the warnings (without their prefix).
pub(crate) fn ini_refused(name: &[u8], v: &[u8]) -> Option<Vec<String>> {
    let encode = matches!(name, b"exif.encode_unicode" | b"exif.encode_jis");
    let decode = matches!(
        name,
        b"exif.decode_unicode_motorola"
            | b"exif.decode_unicode_intel"
            | b"exif.decode_jis_motorola"
            | b"exif.decode_jis_intel"
    );
    if !(encode && !v.is_empty() || decode) {
        return None;
    }
    #[cfg(feature = "mbstring-libmbfl")]
    {
        let item = crate::ext::mbstring::encoding_list_error(v)?;
        Some(vec![
            format!(
                "INI setting contains invalid encoding \"{}\"",
                String::from_utf8_lossy(&item)
            ),
            format!(
                "Illegal encoding ignored: '{}'",
                String::from_utf8_lossy(cstr(v))
            ),
        ])
    }
    #[cfg(not(feature = "mbstring-libmbfl"))]
    {
        None
    }
}

/// PHP_MINFO(exif)'s rows.
pub(crate) fn info_rows() -> Vec<(&'static str, String)> {
    let mb = if cfg!(feature = "mbstring-libmbfl") {
        "enabled"
    } else {
        "disabled"
    };
    vec![
        ("EXIF Support", "enabled".into()),
        ("Supported EXIF Version", "0220".into()),
        ("Supported filetypes", "JPEG, TIFF".into()),
        ("Multibyte decoding support using mbstring", mb.into()),
        (
            "Extended EXIF tag formats",
            "Canon, Casio, Fujifilm, Nikon, Olympus, Samsung, Panasonic, DJI, Sony, Pentax, Minolta, Sigma, Foveon, Kyocera, Ricoh, AGFA, Epson".into(),
        ),
    ]
}

/// exif's constant: EXIF_USE_MBSTRING.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![(
        "EXIF_USE_MBSTRING",
        Value::Int(i64::from(cfg!(feature = "mbstring-libmbfl"))),
    )]
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("exif_tagname", fn_exif_tagname),
            ("exif_read_data", fn_exif_read_data),
            ("exif_thumbnail", fn_exif_thumbnail),
            ("exif_imagetype", fn_exif_imagetype),
        ],
    );
}

#[cfg(test)]
mod tests;
