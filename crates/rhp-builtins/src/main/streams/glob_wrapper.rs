//! main/streams/glob_wrapper.c: `glob://pattern` as a directory stream of
//! the matching names.

use std::any::Any;
use std::rc::Rc;

use super::{StatInfo, StreamWrapper};

/// The `glob://` wrapper: only a directory opener, as in php.
pub struct GlobWrapper;

impl StreamWrapper for GlobWrapper {
    fn exists(&self, _url: &[u8]) -> bool {
        false
    }
    fn get_contents(&self, _url: &[u8]) -> Option<Vec<u8>> {
        None
    }
    fn stat(&self, _url: &[u8]) -> Option<StatInfo> {
        None
    }
    /// `php_glob_stream_opener`: php_glob() of the pattern (no match is an
    /// empty directory); each entry reads as the name after the last slash.
    fn opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        let pattern = url.strip_prefix(b"glob://").unwrap_or(url);
        let (ret, paths) = crate::main::php_glob::php_glob(pattern, 0);
        if ret != 0 && ret != crate::main::php_glob::GLOB_NOMATCH {
            return None;
        }
        Some(
            paths
                .into_iter()
                .map(|p| match p.iter().rposition(|&c| c == b'/') {
                    Some(i) => p[i + 1..].to_vec(),
                    None => p,
                })
                .collect(),
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A fresh `glob://` wrapper instance.
pub fn glob_wrapper() -> Rc<dyn StreamWrapper> {
    Rc::new(GlobWrapper)
}
