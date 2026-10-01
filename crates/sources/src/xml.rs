//! A tiny, dependency-free XML helper for the two well-formed feeds we read
//! (arXiv Atom, TCMB rates).
//!
//! It is deliberately minimal: it extracts the text of repeated elements and
//! the value of an attribute on a repeated element. It is **not** a general XML
//! parser and does not try to be — the alternative would be pulling a full
//! parser and its tree into a hot path for two fixed, machine-generated feeds.
//!
//! Every function is total: a missing tag or a malformed document yields an
//! empty result, never a panic. A feed that stops matching the expected shape
//! therefore produces zero records (recorded as a source failure by the
//! collector), not a crash.

/// The inner text of every `<tag>...</tag>` in `xml`, in document order.
///
/// Matching is non-greedy, so nested repeats (arXiv `<entry>` blocks) each
/// yield their own segment. Entities are decoded for the handful that appear in
/// these feeds.
pub fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        // Guard against `<entry` matching `<entries`: the next character after
        // the tag name must be `>` or whitespace.
        let after = &rest[start + open.len()..];
        if !(after.starts_with('>') || after.starts_with(char::is_whitespace)) {
            rest = &rest[start + open.len()..];
            continue;
        }
        let Some(gt) = rest[start..].find('>') else {
            break;
        };
        let inner_start = start + gt + 1;
        let Some(end) = rest[inner_start..].find(&close) else {
            break;
        };
        out.push(&rest[inner_start..inner_start + end]);
        rest = &rest[inner_start + end + close.len()..];
    }
    out
}

/// The decoded text of the first `<tag>...</tag>`, trimmed.
pub fn text(xml: &str, tag: &str) -> Option<String> {
    elements(xml, tag).first().map(|s| decode(s.trim()))
}

/// The value of `attr` on the first `<tag ...>` opening element.
pub fn attr<'a>(xml: &'a str, tag: &str, attr: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let after = &xml[start + open.len()..];
    if !(after.starts_with('>') || after.starts_with(char::is_whitespace)) {
        return None;
    }
    let gt = xml[start..].find('>')?;
    let header = &xml[start..start + gt];
    let needle = format!("{attr}=\"");
    let a = header.find(&needle)? + needle.len();
    let rest = &header[a..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Decode the XML entities that appear in the feeds we read.
fn decode(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_repeated_elements() {
        let xml = "<feed><entry><title>a</title></entry><entry><title>b</title></entry></feed>";
        let entries = elements(xml, "entry");
        assert_eq!(entries.len(), 2);
        assert_eq!(text(entries[0], "title").as_deref(), Some("a"));
        assert_eq!(text(entries[1], "title").as_deref(), Some("b"));
    }

    #[test]
    fn does_not_confuse_a_prefix_tag() {
        // `<entries>` must not be read as `<entry>`.
        let xml = "<entries><entry><id>1</id></entry></entries>";
        assert_eq!(elements(xml, "entry").len(), 1);
    }

    #[test]
    fn reads_an_attribute() {
        let xml = r#"<link href="https://x/y" rel="alternate"/>"#;
        assert_eq!(attr(xml, "link", "href"), Some("https://x/y"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            text("<t>a &amp; b &lt;c&gt;</t>", "t").as_deref(),
            Some("a & b <c>")
        );
    }

    #[test]
    fn missing_tags_yield_nothing() {
        assert!(elements("<a></a>", "b").is_empty());
        assert!(text("<a></a>", "b").is_none());
        assert!(attr("<a></a>", "b", "c").is_none());
    }
}
