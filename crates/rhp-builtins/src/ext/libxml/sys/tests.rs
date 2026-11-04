use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn error_mirror_matches_the_headers() {
    let c = unsafe { rhp_xml_error_layout };
    let rust = [
        size_of::<XmlErrorRaw>(),
        offset_of!(XmlErrorRaw, domain),
        offset_of!(XmlErrorRaw, code),
        offset_of!(XmlErrorRaw, message),
        offset_of!(XmlErrorRaw, level),
        offset_of!(XmlErrorRaw, file),
        offset_of!(XmlErrorRaw, line),
        offset_of!(XmlErrorRaw, str1),
        offset_of!(XmlErrorRaw, str2),
        offset_of!(XmlErrorRaw, str3),
        offset_of!(XmlErrorRaw, int1),
        offset_of!(XmlErrorRaw, int2),
        offset_of!(XmlErrorRaw, ctxt),
        offset_of!(XmlErrorRaw, node),
    ];
    assert_eq!(c, rust);
}

#[test]
fn tree_mirrors_match_the_headers() {
    let c = unsafe { rhp_xml_tree_layout };
    let rust: Vec<usize> = vec![
        size_of::<XmlNode>(),
        offset_of!(XmlNode, _private),
        offset_of!(XmlNode, type_),
        offset_of!(XmlNode, name),
        offset_of!(XmlNode, children),
        offset_of!(XmlNode, last),
        offset_of!(XmlNode, parent),
        offset_of!(XmlNode, next),
        offset_of!(XmlNode, prev),
        offset_of!(XmlNode, doc),
        offset_of!(XmlNode, ns),
        offset_of!(XmlNode, content),
        offset_of!(XmlNode, properties),
        offset_of!(XmlNode, nsDef),
        offset_of!(XmlNode, psvi),
        offset_of!(XmlNode, line),
        offset_of!(XmlNode, extra),
        offset_of!(XmlAttr, next),
        offset_of!(XmlAttr, doc),
        offset_of!(XmlAttr, ns),
        offset_of!(XmlAttr, atype),
        offset_of!(XmlAttr, psvi),
        size_of::<XmlNs>(),
        offset_of!(XmlNs, next),
        offset_of!(XmlNs, type_),
        offset_of!(XmlNs, href),
        offset_of!(XmlNs, prefix),
        offset_of!(XmlNs, _private),
        offset_of!(XmlNs, context),
        size_of::<XmlDoc>(),
        offset_of!(XmlDoc, compression),
        offset_of!(XmlDoc, standalone),
        offset_of!(XmlDoc, intSubset),
        offset_of!(XmlDoc, extSubset),
        offset_of!(XmlDoc, oldNs),
        offset_of!(XmlDoc, version),
        offset_of!(XmlDoc, encoding),
        offset_of!(XmlDoc, ids),
        offset_of!(XmlDoc, URL),
        offset_of!(XmlDoc, dict),
        offset_of!(XmlDoc, parseFlags),
        offset_of!(XmlDoc, properties),
        size_of::<XmlDtd>(),
        offset_of!(XmlDtd, notations),
        offset_of!(XmlDtd, entities),
        offset_of!(XmlDtd, ExternalID),
        offset_of!(XmlDtd, SystemID),
        offset_of!(XmlDtd, pentities),
        offset_of!(XmlEntity, orig),
        offset_of!(XmlEntity, content),
        offset_of!(XmlEntity, length),
        offset_of!(XmlEntity, etype),
        offset_of!(XmlEntity, ExternalID),
        offset_of!(XmlEntity, SystemID),
        offset_of!(XmlEntity, URI),
        size_of::<XmlNotation>(),
        size_of::<XmlNodeSet>(),
        offset_of!(XmlNodeSet, nodeTab),
        size_of::<XmlXPathObject>(),
        offset_of!(XmlXPathObject, nodesetval),
        offset_of!(XmlXPathObject, boolval),
        offset_of!(XmlXPathObject, floatval),
        offset_of!(XmlXPathObject, stringval),
        offset_of!(XmlXPathObject, index2),
    ];
    assert_eq!(c.to_vec(), rust);
}

/// This test binary is linked straight from the crate, not through the
/// rlib, so it shows whether the build helper's static link holds.
#[test]
fn libxml2_is_linked_statically() {
    if env!("RHP_LIBXML2_LINK_MODE") != "static" {
        return;
    }
    assert!(!super::super::loaded_version().is_empty());
    let exe = std::env::current_exe().unwrap();
    let tool = if cfg!(target_os = "macos") {
        ("otool", vec!["-L"])
    } else {
        ("ldd", vec![])
    };
    let Ok(out) = std::process::Command::new(tool.0)
        .args(&tool.1)
        .arg(&exe)
        .output()
    else {
        return;
    };
    let deps = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{} failed: {deps}", tool.0);
    assert!(!deps.contains("libxml2"), "{deps}");
}
