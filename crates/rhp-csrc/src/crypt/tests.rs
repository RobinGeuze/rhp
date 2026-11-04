use super::*;

fn c(s: &[u8]) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

// php-src ext/standard/config.m4's crypt checks and crypt_*.c self-tests.
#[test]
fn known_vectors() {
    let k = c(b"rasmuslerdorf");
    assert_eq!(des(&k, &c(b"rl")).as_deref(), Some(&b"rl.3StKT.4T8M"[..]));
    assert_eq!(
        des(&k, &c(b"_J9..rasm")).as_deref(),
        Some(&b"_J9..rasmBYk8r9AiWNc"[..])
    );
    assert_eq!(
        blowfish(&k, &c(b"$2a$07$rasmuslerd............")).as_deref(),
        Some(&b"$2a$07$rasmuslerd............nIdrcHdxcUxWomQX9j6kvERCFjTg7Ra"[..])
    );
    assert_eq!(
        sha256(&k, &c(b"$5$rounds=5000$usesomesillystri$")).as_deref(),
        Some(&b"$5$rounds=5000$usesomesillystri$KqJWpanXZHKq2BOB43TSaYhEWsQ1Lr5QNyPCDH/Tp.6"[..])
    );
    assert_eq!(
        sha512(&k, &c(b"$6$rounds=5000$usesomesillystri$")).as_deref(),
        Some(&b"$6$rounds=5000$usesomesillystri$D4IrlXatmP7rx3P3InaxBeoomnAihCKRVQP22JZ6EY47Wc6BkroIuUUBOov1i.S5KPgErtP/EN5mcO.ChWQW21"[..])
    );
}

#[test]
fn failures_are_none() {
    let k = c(b"x");
    assert_eq!(des(&k, &c(b"_........")), None);
    assert_eq!(blowfish(&k, &c(b"$2a$03$1234567891234567891234567")), None);
    assert_eq!(sha512(&k, &c(b"$6$rounds=999$abc$")), None);
    assert_eq!(sha256(&k, &c(b"$5$rounds=1000000000$abc$")), None);
}

// "$6$rounds=1000$" + 16-char salt + "$" + 86 = 118 bytes. php's 123-byte
// buffer hits the same edge with a 9-digit rounds=.
#[test]
fn sha512_result_that_does_not_fit() {
    let (k, s) = (c(b"x"), c(b"$6$rounds=1000$abcdefghijklmnop"));
    let mut buf = [b'X'; 200];
    assert_eq!(crypt_r(php_sha512_crypt_r, &k, &s, &mut buf[..118]), None);
    assert!(buf[..118].iter().all(|&b| b == 0), "wiped");
    assert!(buf[118..].iter().all(|&b| b == b'X'), "C stayed in bounds");

    let hash = crypt_r(php_sha512_crypt_r, &k, &s, &mut [0; 119]).unwrap();
    assert_eq!(hash.len(), 118);
    assert_eq!(sha512(&k, &s), Some(hash));
}

#[test]
fn concurrent_des() {
    let threads: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                for _ in 0..200 {
                    let k = c(format!("pw{i}").as_bytes());
                    let a = des(&k, &c(b"_J9..rasm")).unwrap();
                    let b = des(&k, &c(b"_J9..rasm")).unwrap();
                    assert_eq!(a, b);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}
