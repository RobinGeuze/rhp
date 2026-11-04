use super::*;

#[test]
fn rounding_follows_zend_mm_pages() {
    assert_eq!(rounded(1), 16);
    assert_eq!(rounded(16), 16);
    assert_eq!(rounded(17), 32);
    assert_eq!(rounded(3072), 3072);
    assert_eq!(rounded(3073), 4096);
    assert_eq!(rounded(MAX_LARGE), MAX_LARGE);
    let p = real_page_size();
    assert_eq!(rounded(MAX_LARGE + 1), (MAX_LARGE + p) & !(p - 1));
}

#[test]
fn chunks_and_limit() {
    std::thread::spawn(|| {
        begin_request();
        force_limit(4 * CHUNK_SIZE);
        let s = state();
        assert_eq!(usage(true), CHUNK_SIZE);
        // Large blocks fill chunks; the fifth chunk crosses the limit.
        for _ in 0..4 {
            s.on_alloc(MAX_LARGE);
            assert!(!s.attention());
        }
        assert_eq!(usage(true), 4 * CHUNK_SIZE);
        s.on_alloc(PAGE_SIZE);
        assert!(s.attention());
        let oom = s.take_oom().unwrap();
        assert_eq!(
            oom,
            Oom::Limit {
                limit: 4 * CHUNK_SIZE,
                tried: CHUNK_SIZE
            }
        );
        assert!(oom
            .message()
            .starts_with("Allowed memory size of 8388608 bytes exhausted"));
        // Recorded once per request.
        s.on_alloc(MAX_LARGE);
        assert!(s.take_oom().is_none());
        s.oom_raised();
        assert_eq!(usage(true), 6 * CHUNK_SIZE);
        // Freed chunks stay cached until a lower limit needs them back;
        // the limit can't go below what is in use.
        s.on_free(MAX_LARGE);
        s.on_free(MAX_LARGE);
        s.on_free(PAGE_SIZE);
        assert_eq!(usage(true), 6 * CHUNK_SIZE);
        assert!(!set_limit(2 * CHUNK_SIZE));
        assert_eq!(usage(true), 6 * CHUNK_SIZE);
        assert!(set_limit(4 * CHUNK_SIZE));
        assert_eq!(usage(true), 3 * CHUNK_SIZE);
        s.reset_oom();
        assert!(!reserve(CHUNK_SIZE * 2));
        assert!(s.attention());
        end_request();
    })
    .join()
    .unwrap();
}

#[test]
fn reserve_reports_overflow_and_string_sizes() {
    std::thread::spawn(|| {
        begin_request();
        force_limit(16 * CHUNK_SIZE);
        assert!(reserve_str(1000));
        assert!(!reserve_safe(usize::MAX, 2, 0));
        assert_eq!(
            state().take_oom(),
            Some(Oom::Overflow {
                n: usize::MAX,
                size: 2,
                offset: 0
            })
        );
        assert_eq!(zstr_size(3145728), 3145760);
        end_request();
    })
    .join()
    .unwrap();
}

#[test]
fn huge_growth_and_hard_limit() {
    std::thread::spawn(|| {
        const M: usize = 1 << 20;
        begin_request();
        force_limit(16 * M);
        let s = state();
        // Growing a huge block checks and counts the growth only.
        s.on_alloc(8 * M);
        s.on_realloc(8 * M, 16 * M);
        assert_eq!(
            s.take_oom(),
            Some(Oom::Limit {
                limit: 16 * M,
                tried: 8 * M
            })
        );
        assert_eq!(usage(true), CHUNK_SIZE + 16 * M);
        s.on_realloc(16 * M, 4 * M);
        assert_eq!(usage(true), CHUNK_SIZE + 4 * M);
        s.reset_oom();
        // Below the hard limit (twice the limit) a non-fallible
        // allocation goes ahead; past it, it must not.
        assert!(alloc_check(12 * M));
        assert!(s.take_oom().is_some());
        assert!(!realloc_check(4 * M, 40 * M));
        assert_eq!(
            s.take_oom(),
            Some(Oom::Hard {
                limit: 32 * M,
                tried: 36 * M
            })
        );
        assert!(alloc_check(MAX_LARGE));
        end_request();
    })
    .join()
    .unwrap();
}
