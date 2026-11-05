//! `hebrev()` from ext/standard/string.c: logical Hebrew (ISO-8859-8)
//! text to visual order, with optional line breaking.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

fn isheb(c: u8) -> bool {
    (224..=250).contains(&c)
}
fn isblank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}
fn isnewline(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

/// `hebrev(string $string, int $max_chars_per_line = 0): string`.
fn fn_hebrev(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let max_chars = args.get(1).map(conv::to_int).unwrap_or(0);
    let len = s.len();
    if len == 0 {
        return Ok(Value::str(s));
    }
    // The string is NUL-terminated in php.
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut heb = vec![0u8; len];
    let mut target = len as isize - 1;
    let mut put = |heb: &mut Vec<u8>, c: u8| {
        if target >= 0 {
            heb[target as usize] = c;
        }
        target -= 1;
    };
    let mut t = 0usize;
    let (mut block_start, mut block_end) = (0usize, 0usize);
    let mut hebrew = isheb(s[0]);
    loop {
        if hebrew {
            while (isheb(at(t + 1))
                || isblank(at(t + 1))
                || at(t + 1).is_ascii_punctuation()
                || at(t + 1) == b'\n')
                && block_end < len - 1
            {
                t += 1;
                block_end += 1;
            }
            for i in block_start + 1..=block_end + 1 {
                let c = match s[i - 1] {
                    b'(' => b')',
                    b')' => b'(',
                    b'[' => b']',
                    b']' => b'[',
                    b'{' => b'}',
                    b'}' => b'{',
                    b'<' => b'>',
                    b'>' => b'<',
                    b'\\' => b'/',
                    b'/' => b'\\',
                    c => c,
                };
                put(&mut heb, c);
            }
            hebrew = false;
        } else {
            while !isheb(at(t + 1)) && at(t + 1) != b'\n' && block_end < len - 1 {
                t += 1;
                block_end += 1;
            }
            while (isblank(at(t)) || at(t).is_ascii_punctuation())
                && at(t) != b'/'
                && at(t) != b'-'
                && block_end > block_start
            {
                t -= 1;
                block_end -= 1;
            }
            for i in (block_start + 1..=block_end + 1).rev() {
                put(&mut heb, s[i - 1]);
            }
            hebrew = true;
        }
        block_start = block_end + 1;
        if block_end >= len - 1 {
            break;
        }
    }

    let hat = |heb: &[u8], i: usize| heb.get(i).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(len);
    let (mut begin, mut end) = (len - 1, len - 1);
    loop {
        let mut char_count: i64 = 0;
        while (max_chars == 0 || (max_chars > 0 && char_count < max_chars)) && begin > 0 {
            char_count += 1;
            begin -= 1;
            if isnewline(heb[begin]) {
                while begin > 0 && isnewline(heb[begin - 1]) {
                    begin -= 1;
                    char_count += 1;
                }
                break;
            }
        }
        if max_chars >= 0 && char_count == max_chars {
            // Try to avoid breaking words.
            let (mut new_count, mut new_begin) = (char_count, begin);
            while new_count > 0 {
                let c = hat(&heb, new_begin);
                if isblank(c) || isnewline(c) {
                    break;
                }
                new_begin += 1;
                new_count -= 1;
            }
            if new_count > 0 {
                begin = new_begin;
            }
        }
        let orig_begin = begin;
        if isblank(hat(&heb, begin)) && begin < len {
            heb[begin] = b'\n';
        }
        while begin <= end && isnewline(hat(&heb, begin)) {
            begin += 1;
        }
        for i in begin..=end {
            out.push(hat(&heb, i));
        }
        let mut i = orig_begin;
        while i <= end && isnewline(hat(&heb, i)) {
            out.push(heb[i]);
            i += 1;
        }
        begin = orig_begin;
        if begin == 0 {
            break;
        }
        begin -= 1;
        end = begin;
    }
    out.resize(len, 0);
    Ok(Value::str(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("hebrev", fn_hebrev)]);
}
