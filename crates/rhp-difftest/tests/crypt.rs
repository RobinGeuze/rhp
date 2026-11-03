//! crypt() and password_verify() against live php over generated corpora:
//! random passwords and salts for every crypt algorithm (valid, edge and
//! invalid settings), and mutations of real hashes through password_verify,
//! password_get_info and password_needs_rehash. Skips without php.
//!
//! Work factors stay small: bcrypt costs are only ever 04/05 or invalid, SHA
//! rounds at most 4 digits, argon2 at most m=4096, so no mutation can raise
//! a hash's cost past a few milliseconds.

use rhp_difftest::{diff_source, php_available, php_bytes_literal, run_php, Rng};

const SALT64: &[u8] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ODD: &[u8] = b"$*_:!#{}+-= \n\t\x01\x7f\x80\xa3\xff";

fn pick(rng: &mut Rng, set: &[u8]) -> u8 {
    set[rng.below(set.len())]
}

/// A salt character: mostly from crypt's base64 alphabet.
fn salt_char(rng: &mut Rng) -> u8 {
    if rng.below(10) < 8 {
        pick(rng, SALT64)
    } else {
        pick(rng, ODD)
    }
}

fn salt_chars(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| salt_char(rng)).collect()
}

/// Up to `max - 1` salt characters.
fn some_salt(rng: &mut Rng, max: usize) -> Vec<u8> {
    let n = rng.below(max);
    salt_chars(rng, n)
}

fn one_of<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[rng.below(items.len())]
}

fn password(rng: &mut Rng) -> Vec<u8> {
    match rng.below(4) {
        0 => {
            let n = rng.below(90);
            rng.bytes(n)
        }
        1 => some_salt(rng, 20),
        _ => {
            let n = rng.below(30);
            (0..n).map(|_| b'a' + rng.below(26) as u8).collect()
        }
    }
}

fn des_salt(rng: &mut Rng) -> Vec<u8> {
    let n = one_of(rng, &[0, 1, 2, 2, 2, 2, 13]);
    let mut s = salt_chars(rng, n);
    s.extend(some_salt(rng, 3));
    s
}

fn ext_des_salt(rng: &mut Rng) -> Vec<u8> {
    let mut s = vec![b'_'];
    s.extend(one_of(
        rng,
        &[&b"J9.."[..], b"....", b"/...", b"zz..", b".1.."],
    ));
    let n = one_of(rng, &[4, 4, 4, 3, 2, 8]);
    s.extend(salt_chars(rng, n));
    s
}

fn md5_salt(rng: &mut Rng) -> Vec<u8> {
    let mut s = b"$1$".to_vec();
    s.extend(some_salt(rng, 12));
    if rng.below(2) == 0 {
        s.push(b'$');
        s.extend(some_salt(rng, 5));
    }
    s
}

fn bcrypt_salt(rng: &mut Rng) -> Vec<u8> {
    let mut s = b"$2".to_vec();
    s.push(if rng.below(10) < 8 {
        pick(rng, b"abxy")
    } else {
        pick(rng, b"cgz0$")
    });
    s.push(b'$');
    let cost: &[u8] = if rng.below(10) < 8 {
        one_of(rng, &[&b"04"[..], b"05"])
    } else {
        one_of(rng, &[&b"00"[..], b"03", b"32", b"4$", b"+4", b"a4"])
    };
    s.extend(cost);
    s.push(b'$');
    let n = if rng.below(10) < 8 { 22 } else { rng.below(25) };
    s.extend((0..n).map(|_| {
        if rng.below(40) == 0 {
            pick(rng, b"$*!")
        } else {
            pick(rng, SALT64)
        }
    }));
    s.extend(some_salt(rng, 3));
    s
}

fn sha_salt(rng: &mut Rng) -> Vec<u8> {
    let mut s = if rng.below(2) == 0 {
        b"$5$".to_vec()
    } else {
        b"$6$".to_vec()
    };
    if rng.below(2) == 0 {
        let rounds: &[u8] = one_of(
            rng,
            &[
                &b"1000"[..],
                b"1001",
                b"1234",
                b"2500",
                b"999",
                b"0",
                b"",
                b"1000000000",
                b"18446744073709551617",
                b"-1",
                b" 1000",
                b"+1000",
                b"0x400",
                b"abc",
            ],
        );
        s.extend(b"rounds=");
        s.extend(rounds);
        if rng.below(8) != 0 {
            s.push(b'$');
        }
    }
    s.extend(some_salt(rng, 20));
    if rng.below(2) == 0 {
        s.push(b'$');
    }
    s
}

fn any_salt(rng: &mut Rng) -> Vec<u8> {
    match rng.below(9) {
        0 => des_salt(rng),
        1 => ext_des_salt(rng),
        2 => md5_salt(rng),
        3 | 4 => bcrypt_salt(rng),
        5 | 6 => sha_salt(rng),
        7 => {
            let n = rng.below(8);
            rng.bytes(n)
        }
        _ => {
            let mut s = vec![pick(rng, b"*$_")];
            s.extend(some_salt(rng, 4));
            s
        }
    }
}

/// One line per case; `addcslashes` keeps the output line-based.
fn crypt_script(cases: &[(Vec<u8>, Vec<u8>)]) -> String {
    let mut s = String::from("<?php\n");
    for (i, (pw, salt)) in cases.iter().enumerate() {
        s.push_str(&format!(
            "echo {i}, ' ', addcslashes(crypt({}, {}), \"\\0..\\37\\177..\\377\"), \"\\n\";\n",
            php_bytes_literal(pw),
            php_bytes_literal(salt)
        ));
    }
    s
}

#[test]
fn crypt_matches_php_over_random_corpus() {
    if !php_available() {
        eprintln!("[crypt] no `php` on PATH — skipping");
        return;
    }
    for seed in [0xC0FFEE_u64, 7, 42] {
        let mut rng = Rng::new(seed);
        let cases: Vec<_> = (0..600)
            .map(|_| (password(&mut rng), any_salt(&mut rng)))
            .collect();
        if let Some(why) = diff_source(crypt_script(&cases).as_bytes()).failure() {
            panic!("[crypt seed {seed:#x}] {why}");
        }
    }
}

/// Real hashes from php: crypt() of every algorithm and password_hash().
fn php_hashes(rng: &mut Rng) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut script = String::from("<?php\n");
    let mut pws = Vec::new();
    for i in 0..60 {
        let pw: Vec<u8> = (0..1 + rng.below(20))
            .map(|_| b'a' + rng.below(26) as u8)
            .collect();
        let lit = php_bytes_literal(&pw);
        let salt = |rng: &mut Rng, n| String::from_utf8(salt_chars_strict(rng, n)).unwrap();
        let expr = match i % 12 {
            0 => format!("crypt({lit}, '{}')", salt(rng, 2)),
            1 => format!("crypt({lit}, '_J9..{}')", salt(rng, 4)),
            2 => format!("crypt({lit}, '$1${}$')", salt(rng, 8)),
            3 => format!("crypt({lit}, '$5$rounds=1{:03}${}$')", rng.below(1000), salt(rng, 16)),
            4 => format!("crypt({lit}, '$6${}$')", salt(rng, 12)),
            5 => format!("crypt({lit}, '$2a$04${}')", salt(rng, 22)),
            6 => format!("crypt({lit}, '$2x$04${}')", salt(rng, 22)),
            7 | 8 => format!("password_hash({lit}, PASSWORD_BCRYPT, ['cost' => 4])"),
            9 => format!(
                "password_hash({lit}, PASSWORD_ARGON2I, ['memory_cost' => {}, 'time_cost' => {}, 'threads' => {}])",
                [1024, 2048, 4096][rng.below(3)],
                1 + rng.below(2),
                1 + rng.below(2)
            ),
            _ => format!(
                "password_hash({lit}, PASSWORD_ARGON2ID, ['memory_cost' => {}, 'time_cost' => {}, 'threads' => {}])",
                [1024, 2048, 4096][rng.below(3)],
                1 + rng.below(2),
                1 + rng.below(2)
            ),
        };
        script.push_str(&format!("echo {expr}, \"\\n\";\n"));
        pws.push(pw);
    }
    let out = run_php(script.as_bytes()).expect("php hashes");
    out.split(|&b| b == b'\n')
        .zip(pws)
        .map(|(h, pw)| (pw, h.to_vec()))
        .collect()
}

fn salt_chars_strict(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| pick(rng, SALT64)).collect()
}

/// Where a byte may change without raising the work factor: past bcrypt's
/// cost, and anywhere else (SHA rounds and argon2 parameters stay at most
/// 4 digits, so a flipped digit stays cheap).
fn safe_start(h: &[u8]) -> usize {
    if h.starts_with(b"$2") {
        7.min(h.len())
    } else {
        0
    }
}

fn mutate(rng: &mut Rng, h: &[u8]) -> Vec<u8> {
    let mut m = h.to_vec();
    let start = safe_start(h);
    match rng.below(8) {
        0 | 1 if m.len() > start => {
            let i = start + rng.below(m.len() - start);
            m[i] = if rng.below(3) == 0 {
                pick(rng, ODD)
            } else {
                pick(rng, SALT64)
            };
        }
        2 => m.truncate(rng.below(m.len() + 1)),
        3 => m.extend(some_salt(rng, 3)),
        4 if m.len() > start => {
            let i = start + rng.below(m.len() - start);
            m.insert(i, 0);
        }
        5 => {
            // Another variant of the same family.
            for (from, to) in [
                (&b"$2a$"[..], &b"$2y$"[..]),
                (b"$2y$", b"$2x$"),
                (b"$2x$", b"$2b$"),
                (b"$argon2id$", b"$argon2i$"),
                (b"$argon2i$", b"$argon2id$"),
                (b"$5$", b"$6$"),
                (b"$6$", b"$5$"),
                (b"$1$", b"$2$"),
            ] {
                if m.starts_with(from) {
                    m.splice(..from.len(), to.iter().copied());
                    break;
                }
            }
        }
        6 => {
            // argon2 parameter syntax that libargon2 refuses.
            let s = String::from_utf8_lossy(&m).into_owned();
            let s = match rng.below(4) {
                0 => s.replacen("v=19$", "", 1),
                1 => s.replacen("v=19", "v=16", 1),
                2 => s.replacen(",p=", ",x=", 1),
                _ => s.replacen("m=", "M=", 1),
            };
            m = s.into_bytes();
        }
        _ => {}
    }
    m
}

#[test]
fn password_functions_match_php_over_hash_mutations() {
    if !php_available() {
        eprintln!("[password] no `php` on PATH — skipping");
        return;
    }
    for seed in [0xC0FFEE_u64, 7] {
        let mut rng = Rng::new(seed);
        let hashes = php_hashes(&mut rng);
        let mut script = String::from("<?php\n");
        for (pw, h) in &hashes {
            for k in 0..4 {
                let m = if k == 0 {
                    h.clone()
                } else {
                    mutate(&mut rng, h)
                };
                let (pw, m) = (php_bytes_literal(pw), php_bytes_literal(&m));
                script.push_str(&format!(
                    "echo serialize([password_verify({pw}, {m}), password_verify('x', {m}), \
                     password_get_info({m}), password_needs_rehash({m}, PASSWORD_BCRYPT, ['cost' => 4]), \
                     password_needs_rehash({m}, PASSWORD_ARGON2I, ['memory_cost' => 1024, 'time_cost' => 1]), \
                     password_needs_rehash({m}, PASSWORD_ARGON2ID)]), \"\\n\";\n"
                ));
            }
        }
        if let Some(why) = diff_source(script.as_bytes()).failure() {
            panic!("[password seed {seed:#x}] {why}");
        }
    }
}
