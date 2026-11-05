//! HTTP header types
//!
//! This module provides [`HeaderName`], [`HeaderMap`], [`OrigHeaderMap`], [`HeaderCaseName`], and a
//! number of types used for interacting with `HeaderMap`. These types allow representing both
//! HTTP/1 and HTTP/2 headers.

use std::collections::BTreeMap;

use bytes::Bytes;
pub use http::header::*;
use netty::ext::OnPreserveHeaderCallback;

/// Trait for types that can be converted into an [`HeaderCaseName`] (case-preserved header).
///
/// This trait is sealed, so only known types can implement it.
/// Supported types:
/// - `&'static str`
/// - `String`
/// - `Bytes`
/// - `HeaderName`
/// - `&HeaderName`
/// - `HeaderCaseName`
/// - `&HeaderCaseName`
pub trait IntoHeaderCaseName: sealed::Sealed {
    /// Converts the type into an [`HeaderCaseName`].
    fn into_header_case_name(self) -> HeaderCaseName;
}

/// A map from header names to their original casing as received in an HTTP message.
///
/// [`OrigHeaderMap`] not only preserves the original case of each header name as it appeared
/// in the request or response, but also maintains the insertion order of headers. This makes
/// it suitable for use cases where the order of headers matters, such as HTTP/1.x message
/// serialization, proxying, or reproducing requests/responses exactly as received.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrigHeaderMap(HeaderMap<HeaderCaseName>);

// ===== impl OrigHeaderMap =====

impl OrigHeaderMap {
    /// Creates a new, empty [`OrigHeaderMap`].
    #[inline]
    pub fn new() -> Self {
        Self(HeaderMap::default())
    }

    /// Creates an empty [`OrigHeaderMap`] with the specified capacity.
    #[inline]
    pub fn with_capacity(size: usize) -> Self {
        Self(HeaderMap::with_capacity(size))
    }

    /// Insert a new header name into the collection.
    ///
    /// If the map did not previously have this key present, then `false` is
    /// returned.
    ///
    /// If the map did have this key present, the new value is pushed to the end
    /// of the list of values currently associated with the key. The key is not
    /// updated, though; this matters for types that can be `==` without being
    /// identical.
    #[inline]
    pub fn insert<N>(&mut self, orig: N) -> bool
    where
        N: IntoHeaderCaseName,
    {
        let header_case_name = orig.into_header_case_name();
        match &header_case_name.inner {
            Repr::Cased(bytes) => HeaderName::from_bytes(bytes)
                .map(|header_name| self.0.append(header_name, header_case_name))
                .unwrap_or(false),
            Repr::Standard(header_name) => self.0.append(header_name.clone(), header_case_name),
        }
    }

    /// Extends the map with all entries from another [`OrigHeaderMap`], preserving order.
    #[inline]
    pub fn extend(&mut self, iter: OrigHeaderMap) {
        self.0.extend(iter.0);
    }

    /// Returns the number of headers stored in the map.
    ///
    /// This number represents the total number of **values** stored in the map.
    /// This number can be greater than or equal to the number of **keys**
    /// stored given that a single key may have more than one associated value.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the map contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns an iterator over all header names and their original spellings, in insertion order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderCaseName)> {
        self.0.iter()
    }
}

impl OnPreserveHeaderCallback for OrigHeaderMap {
    fn call(&self, headers: &mut HeaderMap) {
        if headers.len() <= 1 || self.0.is_empty() {
            return;
        }

        // Create a new header map to store the sorted headers
        let mut sorted_headers = HeaderMap::with_capacity(headers.keys_len());

        // First insert headers in the specified order
        for name in self.0.keys() {
            for value in headers.get_all(name) {
                sorted_headers.append(name.clone(), value.clone());
            }
            headers.remove(name);
        }

        // Then insert any remaining headers that were not ordered
        let mut prev_name: Option<HeaderName> = None;
        for (name, value) in headers.drain() {
            match (name, &prev_name) {
                (Some(name), _) => {
                    prev_name.replace(name.clone());
                    sorted_headers.insert(name, value);
                }
                (None, Some(prev_name)) => {
                    sorted_headers.append(prev_name, value);
                }
                _ => {}
            }
        }

        std::mem::swap(headers, &mut sorted_headers);
    }

    fn call_visit(
        &self,
        headers: &mut HeaderMap,
        dst: &mut dyn FnMut(&dyn AsRef<[u8]>, &http::HeaderValue),
    ) {
        // First, sort headers according to the order defined in this map
        for (name, case_name) in self.iter() {
            for value in headers.get_all(name) {
                dst(case_name, value);
            }

            headers.remove(name);
        }

        // After processing all ordered headers, append any remaining headers
        let mut prev_name: Option<HeaderCaseName> = None;
        for (name, value) in headers.drain() {
            match (name, &prev_name) {
                (Some(name), _) => {
                    dst(&name, &value);
                    prev_name.replace(name.into_header_case_name());
                }
                (None, Some(prev_name)) => {
                    dst(prev_name, &value);
                }
                _ => (),
            };
        }
    }
}

impl<'a> IntoIterator for &'a OrigHeaderMap {
    type Item = (&'a HeaderName, &'a HeaderCaseName);
    type IntoIter = <&'a HeaderMap<HeaderCaseName> as IntoIterator>::IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for OrigHeaderMap {
    type Item = (Option<HeaderName>, HeaderCaseName);
    type IntoIter = <HeaderMap<HeaderCaseName> as IntoIterator>::IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl_request_config_value!(OrigHeaderMap);

/// Request extension that carries the header block in wire order.
///
/// A caller that must reproduce an exact HTTP/1.1 header block inserts this type into the
/// request extensions. `wreq` writes each entry in the listed order with the listed name
/// bytes, and then writes the `HeaderMap` entries that this list does not cover. The
/// HTTP/2 encoder keeps its own order, so this type only changes the header set there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderedHeaders {
    entries: Vec<(Vec<u8>, HeaderValue)>,
}

impl OrderedHeaders {
    /// Creates a new, empty [`OrderedHeaders`].
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new [`OrderedHeaders`] with the specified capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Appends one header with the given name bytes and no deduplication.
    ///
    /// The name bytes are the wire spelling, so `X-Trace-Id` and `x-trace-id` stay two
    /// separate entries with their own casing.
    #[inline]
    pub fn push(&mut self, name: &[u8], value: HeaderValue) {
        self.entries.push((name.to_vec(), value));
    }

    /// Returns the number of listed headers.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if no header is listed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Counts each listed header by name and value bytes.
    ///
    /// A header map entry that matches a listed entry is a duplicate of it, not an extra
    /// header, so it must not be written twice.
    fn listed_counts(&self) -> BTreeMap<(String, Vec<u8>), usize> {
        let mut counts = BTreeMap::new();
        for (name, value) in &self.entries {
            if let Ok(name) = HeaderName::from_bytes(name) {
                *counts
                    .entry((name.as_str().to_string(), value.as_bytes().to_vec()))
                    .or_insert(0) += 1;
            }
        }
        counts
    }
}

/// Writes every entry of `taken` that the listed counts do not cover.
fn write_unlisted(
    mut counts: BTreeMap<(String, Vec<u8>), usize>,
    mut taken: HeaderMap,
    mut visit: impl FnMut(&HeaderName, &HeaderValue),
) {
    let mut previous: Option<HeaderName> = None;
    for (name, value) in taken.drain() {
        if let Some(name) = name {
            previous = Some(name);
        }
        let Some(name) = previous.as_ref() else {
            continue;
        };
        let key = (name.as_str().to_string(), value.as_bytes().to_vec());
        match counts.get_mut(&key) {
            Some(count) if *count > 0 => *count -= 1,
            _ => visit(name, &value),
        }
    }
}

impl OnPreserveHeaderCallback for OrderedHeaders {
    fn call(&self, headers: &mut HeaderMap) {
        if self.entries.is_empty() {
            return;
        }
        let counts = self.listed_counts();
        let taken = std::mem::take(headers);
        let mut rebuilt = HeaderMap::with_capacity(taken.len() + self.entries.len());
        for (raw_name, value) in &self.entries {
            if let Ok(name) = HeaderName::from_bytes(raw_name) {
                rebuilt.append(name, value.clone());
            }
        }
        write_unlisted(counts, taken, |name, value| {
            rebuilt.append(name.clone(), value.clone());
        });
        *headers = rebuilt;
    }

    fn call_visit(
        &self,
        headers: &mut HeaderMap,
        dst: &mut dyn FnMut(&dyn AsRef<[u8]>, &HeaderValue),
    ) {
        for (raw_name, value) in &self.entries {
            dst(raw_name, value);
        }
        let counts = self.listed_counts();
        write_unlisted(counts, std::mem::take(headers), |name, value| {
            dst(name, value);
        });
    }
}

/// An HTTP header name with both normalized and original casing.
///
/// While HTTP headers are case-insensitive, this type stores both
/// the canonical [`HeaderName`] and the original casing as received,
/// useful for preserving header order and formatting in proxies,
/// debugging, or exact HTTP message reproduction.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct HeaderCaseName {
    inner: Repr,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum Repr {
    Cased(Bytes),
    Standard(HeaderName),
}

impl AsRef<[u8]> for HeaderCaseName {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        match &self.inner {
            Repr::Standard(name) => name.as_ref(),
            Repr::Cased(orig) => orig.as_ref(),
        }
    }
}

impl IntoHeaderCaseName for &'static str {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        Bytes::from_static(self.as_bytes()).into_header_case_name()
    }
}

impl IntoHeaderCaseName for String {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        Bytes::from(self).into_header_case_name()
    }
}

impl IntoHeaderCaseName for Bytes {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        HeaderCaseName {
            inner: Repr::Cased(self),
        }
    }
}

impl IntoHeaderCaseName for &HeaderName {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        HeaderCaseName {
            inner: Repr::Standard(self.clone()),
        }
    }
}

impl IntoHeaderCaseName for HeaderName {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        HeaderCaseName {
            inner: Repr::Standard(self),
        }
    }
}

impl IntoHeaderCaseName for HeaderCaseName {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        self
    }
}

impl IntoHeaderCaseName for &HeaderCaseName {
    #[inline]
    fn into_header_case_name(self) -> HeaderCaseName {
        self.clone()
    }
}

mod sealed {

    use bytes::Bytes;
    use http::HeaderName;

    use crate::header::HeaderCaseName;

    pub trait Sealed {}

    impl Sealed for &'static str {}
    impl Sealed for String {}
    impl Sealed for Bytes {}
    impl Sealed for &HeaderName {}
    impl Sealed for HeaderName {}
    impl Sealed for &HeaderCaseName {}
    impl Sealed for HeaderCaseName {}
}

#[cfg(test)]
mod test {
    use http::{HeaderMap, HeaderName, HeaderValue};
    use netty::ext::OnPreserveHeaderCallback;

    use super::OrigHeaderMap;

    /// Returns a view of all spellings associated with that header name,
    /// in the order they were found.
    #[inline]
    pub(crate) fn get_all<'a>(
        orig_headers: &'a OrigHeaderMap,
        name: &HeaderName,
    ) -> impl Iterator<Item = impl AsRef<[u8]> + 'a> + 'a {
        orig_headers.0.get_all(name).into_iter()
    }

    #[test]
    fn test_header_order() {
        let mut headers = OrigHeaderMap::new();

        // Insert headers with different cases and order
        headers.insert("X-Test");
        headers.insert("X-Another");
        headers.insert("x-test2");

        // Check order and case
        let mut iter = headers.iter();
        assert_eq!(iter.next().unwrap().1.as_ref(), b"X-Test");
        assert_eq!(iter.next().unwrap().1.as_ref(), b"X-Another");
        assert_eq!(iter.next().unwrap().1.as_ref(), b"x-test2");
    }

    #[test]
    fn test_extend_preserves_order() {
        use super::OrigHeaderMap;

        let mut map1 = OrigHeaderMap::new();
        map1.insert("A-Header");
        map1.insert("B-Header");

        let mut map2 = OrigHeaderMap::new();
        map2.insert("C-Header");
        map2.insert("D-Header");

        map1.extend(map2);

        let names: Vec<_> = map1.iter().map(|(_, orig)| orig.as_ref()).collect();
        assert_eq!(
            names,
            vec![b"A-Header", b"B-Header", b"C-Header", b"D-Header"]
        );
    }

    #[test]
    fn test_header_case() {
        let mut headers = OrigHeaderMap::new();

        // Insert headers with different cases
        headers.insert("X-Test");
        headers.insert("x-test");

        // Check that both headers are stored
        let all_x_test: Vec<_> = get_all(&headers, &"X-Test".parse().unwrap()).collect();
        assert_eq!(all_x_test.len(), 2);
        assert!(all_x_test.iter().any(|v| v.as_ref() == b"X-Test"));
        assert!(all_x_test.iter().any(|v| v.as_ref() == b"x-test"));
    }

    #[test]
    fn test_header_multiple_cases() {
        let mut headers = OrigHeaderMap::new();

        // Insert multiple headers with the same name but different cases
        headers.insert("X-test");
        headers.insert("x-test");
        headers.insert("X-test");

        // Check that all variations are stored
        let all_x_test: Vec<_> = get_all(&headers, &"x-test".parse().unwrap()).collect();
        assert_eq!(all_x_test.len(), 3);
        assert!(all_x_test.iter().any(|v| v.as_ref() == b"X-test"));
        assert!(all_x_test.iter().any(|v| v.as_ref() == b"x-test"));
        assert!(all_x_test.iter().any(|v| v.as_ref() == b"X-test"));
    }

    #[test]
    fn test_sort_headers_preserves_multiple_cookie_values() {
        // Create original header map for ordering
        let mut orig_headers = OrigHeaderMap::new();
        orig_headers.insert("Cookie");
        orig_headers.insert("User-Agent");
        orig_headers.insert("Accept");

        // Create headers with multiple Cookie values
        let mut headers = HeaderMap::new();

        // Add multiple Cookie headers (this simulates how cookies are often sent)
        headers.append("cookie", HeaderValue::from_static("session=abc123"));
        headers.append("cookie", HeaderValue::from_static("theme=dark"));
        headers.append("cookie", HeaderValue::from_static("lang=en"));

        // Add other headers
        headers.insert("user-agent", HeaderValue::from_static("Mozilla/5.0"));
        headers.insert("accept", HeaderValue::from_static("text/html"));
        headers.insert("host", HeaderValue::from_static("example.com"));

        // Record original cookie values for comparison
        let original_cookies: Vec<_> = headers
            .get_all("cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();

        // Sort headers according to orig_headers order
        orig_headers.call(&mut headers);

        // Verify all cookie values are preserved
        let sorted_cookies: Vec<_> = headers
            .get_all("cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();

        assert_eq!(
            original_cookies.len(),
            sorted_cookies.len(),
            "Cookie count should be preserved"
        );
        assert_eq!(original_cookies.len(), 3, "Should have 3 cookie values");

        // Verify all original cookies are still present (order might change but content preserved)
        for original_cookie in &original_cookies {
            assert!(
                sorted_cookies.contains(original_cookie),
                "Cookie '{original_cookie}' should be preserved"
            );
        }

        // Verify header ordering - Cookie should come first
        let header_names: Vec<_> = headers.keys().collect();
        assert_eq!(
            header_names[0].as_str(),
            "cookie",
            "Cookie should be first header"
        );

        // Verify all headers are preserved
        assert_eq!(
            headers.len(),
            6,
            "Should have 6 total header values (3 cookies + 3 others)"
        );
        assert!(headers.contains_key("user-agent"));
        assert!(headers.contains_key("accept"));
        assert!(headers.contains_key("host"));
    }

    #[test]
    fn test_sort_headers_multiple_values_different_headers() {
        let mut orig_headers = OrigHeaderMap::new();
        orig_headers.insert("Accept");
        orig_headers.insert("Cookie");

        let mut headers = HeaderMap::new();

        // Multiple Accept headers
        headers.append("accept", HeaderValue::from_static("text/html"));
        headers.append("accept", HeaderValue::from_static("application/json"));

        // Multiple Cookie headers
        headers.append("cookie", HeaderValue::from_static("a=1"));
        headers.append("cookie", HeaderValue::from_static("b=2"));

        // Single header
        headers.insert("host", HeaderValue::from_static("example.com"));

        let total_before = headers.len();

        orig_headers.call(&mut headers);

        // Verify all values preserved
        assert_eq!(
            headers.len(),
            total_before,
            "Total header count should be preserved"
        );
        assert_eq!(
            headers.get_all("accept").iter().count(),
            2,
            "Accept headers should be preserved"
        );
        assert_eq!(
            headers.get_all("cookie").iter().count(),
            2,
            "Cookie headers should be preserved"
        );
        assert_eq!(
            headers.get_all("host").iter().count(),
            1,
            "Host header should be preserved"
        );
    }
}
