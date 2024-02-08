mod header_name;
mod header_value;
mod header_values;
mod known_header_name;
mod unknown_header_name;

pub use header_name::HeaderName;
pub use header_value::HeaderValue;
pub use header_values::HeaderValues;
pub use known_header_name::KnownHeaderName;

use header_name::HeaderNameInner;
use memchr::memmem::Finder;
use unknown_header_name::UnknownHeaderName;

use hashbrown::{
    hash_map::{self, Entry},
    HashMap,
};
use smartcow::SmartCow;
use std::collections::{
    btree_map::{self, Entry as BTreeEntry},
    BTreeMap,
};
use std::{
    fmt::{self, Debug, Display, Formatter},
    hash::Hasher,
};

use crate::Error;

/// Trillium's header map type
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[must_use]
pub struct Headers {
    known: BTreeMap<KnownHeaderName, HeaderValues>,
    unknown: HashMap<UnknownHeaderName<'static>, HeaderValues>,
}

#[cfg(feature = "serde")]
impl serde::Serialize for Headers {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, values) in self {
            map.serialize_entry(&key, values)?;
        }
        map.end()
    }
}

impl Display for Headers {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (n, v) in self {
            for v in v {
                f.write_fmt(format_args!("{n}: {v}\r\n"))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParseError;
impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("parse error")
    }
}

fn is_tchar(c: u8) -> bool {
    matches!(
        c,
        b'a'..=b'z'
        | b'A'..=b'Z'
        | b'0'..=b'9'
        | b'!'
        | b'#'
        | b'$'
        | b'%'
        | b'&'
        | b'\''
        | b'*'
        | b'+'
        | b'-'
        | b'.'
        | b'^'
        | b'_'
        | b'`'
        | b'|'
        | b'~'
    )
}

impl Headers {
    #[doc(hidden)]
    pub fn extend_parse(&mut self, bytes: &[u8]) -> Result<usize, Error> {
        let newlines = Finder::new(b"\r\n").find_iter(bytes).collect::<Vec<_>>();
        //        self.reserve(newlines.len().saturating_sub(1));
        let mut new_header_count = 0;
        let mut last_line = 0;
        for newline in newlines {
            if newline == last_line {
                continue;
            }

            let token_start = last_line;
            let mut token_end = token_start;
            while is_tchar(bytes[token_end]) {
                token_end += 1;
            }

            let header_name = HeaderName::parse(&bytes[token_start..token_end])?.to_owned();

            if bytes[token_end] != b':' {
                return Err(Error::InvalidHead);
            }

            let mut value_start = token_end + 1;
            while (bytes[value_start] as char).is_whitespace() {
                value_start += 1;
            }

            let header_value = HeaderValue::parse(&bytes[value_start..newline]);
            self.append(header_name, header_value);
            new_header_count += 1;
            last_line = newline + 2;
        }
        Ok(new_header_count)
    }

    #[cfg(feature = "parse")]
    #[doc(hidden)]
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut headers = Headers::new();
        headers.extend_parse(bytes)?;
        Ok(headers)
    }

    /// Construct a new headers with a default capacity
    pub fn new() -> Self {
        Self::default()
    }

    /// Return an iterator over borrowed header names and header
    /// values. First yields the known headers and then the unknown
    /// headers, if any.
    pub fn iter(&self) -> Iter<'_> {
        self.into()
    }

    /// Are there zero headers?
    pub fn is_empty(&self) -> bool {
        self.known.is_empty() && self.unknown.is_empty()
    }

    /// How many unique [`HeaderName`] have been added to these [`Headers`]?
    /// Note that each header name may have more than one [`HeaderValue`].
    pub fn len(&self) -> usize {
        self.known.len() + self.unknown.len()
    }

    /// add the header value or header values into this header map. If
    /// there is already a header with the same name, the new values
    /// will be added to the existing ones. To replace any existing
    /// values, use [`Headers::insert`]
    pub fn append(&mut self, name: impl Into<HeaderName<'static>>, value: impl Into<HeaderValues>) {
        let value = value.into();
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => match self.known.entry(known) {
                BTreeEntry::Occupied(mut o) => {
                    o.get_mut().extend(value);
                }
                BTreeEntry::Vacant(v) => {
                    v.insert(value);
                }
            },

            HeaderNameInner::UnknownHeader(unknown) => match self.unknown.entry(unknown) {
                Entry::Occupied(mut o) => {
                    o.get_mut().extend(value);
                }
                Entry::Vacant(v) => {
                    v.insert(value);
                }
            },
        }
    }

    /// A slightly more efficient way to combine two [`Headers`] than
    /// using [`Extend`]
    pub fn append_all(&mut self, other: Headers) {
        for (name, value) in other.known {
            match self.known.entry(name) {
                BTreeEntry::Occupied(mut entry) => {
                    entry.get_mut().extend(value);
                }
                BTreeEntry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }

        for (name, value) in other.unknown {
            match self.unknown.entry(name) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().extend(value);
                }
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }
    }

    /// Combine two [`Headers`], replacing any existing header values
    pub fn insert_all(&mut self, other: Headers) {
        for (name, value) in other.known {
            self.known.insert(name, value);
        }

        for (name, value) in other.unknown {
            self.unknown.insert(name, value);
        }
    }

    /// Add a header value or header values into this header map. If a
    /// header already exists with the same name, it will be
    /// replaced. To combine, see [`Headers::append`]
    pub fn insert(&mut self, name: impl Into<HeaderName<'static>>, value: impl Into<HeaderValues>) {
        let value = value.into();
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => {
                self.known.insert(known, value);
            }

            HeaderNameInner::UnknownHeader(unknown) => {
                self.unknown.insert(unknown, value);
            }
        }
    }

    /// Add a header value or header values into this header map if
    /// and only if there is not already a header with the same name.
    pub fn try_insert(
        &mut self,
        name: impl Into<HeaderName<'static>>,
        value: impl Into<HeaderValues>,
    ) {
        let value = value.into();
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => {
                self.known.entry(known).or_insert(value);
            }

            HeaderNameInner::UnknownHeader(unknown) => {
                self.unknown.entry(unknown).or_insert(value);
            }
        }
    }

    /// Retrieves a &str header value if there is at least one header
    /// in the map with this name. If there are several headers with
    /// the same name, this follows the behavior defined at
    /// [`HeaderValues::one`]. Returns None if there is no header with
    /// the provided header name.
    pub fn get_str<'a>(&self, name: impl Into<HeaderName<'a>>) -> Option<&str> {
        self.get_values(name).and_then(HeaderValues::as_str)
    }

    pub(crate) fn get_lower<'a>(&self, name: impl Into<HeaderName<'a>>) -> Option<SmartCow<'_>> {
        self.get_values(name).and_then(HeaderValues::as_lower)
    }

    /// Retrieves a singular header value from this header map. If
    /// there are several headers with the same name, this follows the
    /// behavior defined at [`HeaderValues::one`]. Returns None if there is no header with the provided header name
    pub fn get<'a>(&self, name: impl Into<HeaderName<'a>>) -> Option<&HeaderValue> {
        self.get_values(name).and_then(HeaderValues::one)
    }

    /// Takes all headers with the provided header name out of this
    /// header map and returns them. Returns None if the header did
    /// not have an entry in this map.
    pub fn remove<'a>(&mut self, name: impl Into<HeaderName<'a>>) -> Option<HeaderValues> {
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => self.known.remove(&known),
            HeaderNameInner::UnknownHeader(unknown) => self.unknown.remove(&&unknown),
        }
    }

    /// Retrieves a reference to all header values with the provided
    /// header name. If you expect there to be only one value, use
    /// [`Headers::get`].
    pub fn get_values<'a>(&self, name: impl Into<HeaderName<'a>>) -> Option<&HeaderValues> {
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => self.known.get(&known),
            HeaderNameInner::UnknownHeader(unknown) => self.unknown.get(&&unknown),
        }
    }

    /// Predicate function to check whether this header map contains
    /// the provided header name. If you are using this to
    /// conditionally insert a value, consider using
    /// [`Headers::try_insert`] instead.
    pub fn has_header<'a>(&self, name: impl Into<HeaderName<'a>>) -> bool {
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => self.known.contains_key(&known),
            HeaderNameInner::UnknownHeader(unknown) => self.unknown.contains_key(&unknown),
        }
    }

    /// Convenience function to check whether the value contained in
    /// this header map for the provided name is
    /// ascii-case-insensitively equal to the provided comparison
    /// &str. Returns false if there is no value for the name
    pub fn eq_ignore_ascii_case<'a>(
        &'a self,
        name: impl Into<HeaderName<'a>>,
        needle: &str,
    ) -> bool {
        self.get_str(name)
            .is_some_and(|v| v.eq_ignore_ascii_case(needle))
    }

    /// Deprecated because is likely not what you want. It is rarely the case that headers should
    /// be searched for a matching string instead of carefully parsed according to the appropriate
    /// header rule. Naive string matching on headers without regard to header structure is a
    /// possible source of spec noncompliance or occasionally security vulnerability, so trillium
    /// does not go out of its way to facilitate that.
    #[deprecated = "Please open an issue if this behavior is important to you. \
See documentation for deprecation rationale"]
    pub fn contains_ignore_ascii_case<'a>(
        &self,
        name: impl Into<HeaderName<'a>>,
        needle: &str,
    ) -> bool {
        self.get_str(name).is_some_and(|h| {
            let needle = if needle.chars().all(|c| c.is_ascii_lowercase()) {
                SmartCow::Borrowed(needle)
            } else {
                SmartCow::Owned(needle.chars().map(|c| c.to_ascii_lowercase()).collect())
            };

            if h.chars().all(|c| c.is_ascii_lowercase()) {
                h.contains(&*needle)
            } else {
                h.to_ascii_lowercase().contains(&*needle)
            }
        })
    }

    /// Chainable method to insert a header
    pub fn with_inserted_header(
        mut self,
        name: impl Into<HeaderName<'static>>,
        values: impl Into<HeaderValues>,
    ) -> Self {
        self.insert(name, values);
        self
    }

    /// Chainable method to append a header
    pub fn with_appended_header(
        mut self,
        name: impl Into<HeaderName<'static>>,
        values: impl Into<HeaderValues>,
    ) -> Self {
        self.append(name, values);
        self
    }

    /// Chainable method to remove a header
    pub fn without_header(mut self, name: impl Into<HeaderName<'static>>) -> Self {
        self.remove(name);
        self
    }

    /// Chainable method to remove multiple headers by name
    pub fn without_headers<I, H>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = H>,
        H: Into<HeaderName<'static>>,
    {
        self.remove_all(names);
        self
    }

    /// remove multiple headers by name
    pub fn remove_all<I, H>(&mut self, names: I)
    where
        I: IntoIterator<Item = H>,
        H: Into<HeaderName<'static>>,
    {
        for header in names {
            self.remove(header.into());
        }
    }

    /// if a key does not exist already, execute the provided function and insert a value
    ///
    /// this can be useful to avoid calculating an unnecessary header value, or checking for the
    /// presence of a key before insertion
    pub fn try_insert_with<F, V>(&mut self, name: impl Into<HeaderName<'static>>, values_fn: F)
    where
        F: Fn() -> V,
        V: Into<HeaderValues>,
    {
        match name.into().0 {
            HeaderNameInner::KnownHeader(known) => {
                self.known
                    .entry(known)
                    .or_insert_with(|| values_fn().into());
            }

            HeaderNameInner::UnknownHeader(unknown) => {
                self.unknown
                    .entry(unknown)
                    .or_insert_with(|| values_fn().into());
            }
        }
    }
}

impl<HN, HV> Extend<(HN, HV)> for Headers
where
    HN: Into<HeaderName<'static>>,
    HV: Into<HeaderValues>,
{
    fn extend<T: IntoIterator<Item = (HN, HV)>>(&mut self, iter: T) {
        for (name, values) in iter {
            self.append(name, values);
        }
    }
}

impl<HN, HV> FromIterator<(HN, HV)> for Headers
where
    HN: Into<HeaderName<'static>>,
    HV: Into<HeaderValues>,
{
    fn from_iter<T: IntoIterator<Item = (HN, HV)>>(iter: T) -> Self {
        let iter = iter.into_iter();
        let mut headers = Self::new();
        for (name, values) in iter {
            headers.append(name, values);
        }

        headers
    }
}

#[derive(Default)]
struct DirectHasher(u8);

impl Hasher for DirectHasher {
    fn write(&mut self, _: &[u8]) {
        unreachable!("KnownHeaderName calls write_u64");
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0 = i;
    }

    #[inline]
    fn finish(&self) -> u64 {
        u64::from(self.0)
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = (HeaderName<'a>, &'a HeaderValues);

    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

#[derive(Debug)]
pub struct IntoIter {
    known: btree_map::IntoIter<KnownHeaderName, HeaderValues>,
    unknown: hash_map::IntoIter<UnknownHeaderName<'static>, HeaderValues>,
}

impl Iterator for IntoIter {
    type Item = (HeaderName<'static>, HeaderValues);

    fn next(&mut self) -> Option<Self::Item> {
        let IntoIter { known, unknown } = self;
        known
            .next()
            .map(|(k, v)| (HeaderName::from(k), v))
            .or_else(|| unknown.next().map(|(k, v)| (HeaderName::from(k), v)))
    }
}
impl From<Headers> for IntoIter {
    fn from(value: Headers) -> Self {
        Self {
            known: value.known.into_iter(),
            unknown: value.unknown.into_iter(),
        }
    }
}

#[derive(Debug)]
pub struct Iter<'a> {
    known: btree_map::Iter<'a, KnownHeaderName, HeaderValues>,
    unknown: hash_map::Iter<'a, UnknownHeaderName<'static>, HeaderValues>,
}

impl<'a> From<&'a Headers> for Iter<'a> {
    fn from(value: &'a Headers) -> Self {
        Iter {
            known: value.known.iter(),
            unknown: value.unknown.iter(),
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (HeaderName<'a>, &'a HeaderValues);

    fn next(&mut self) -> Option<Self::Item> {
        let Iter { known, unknown } = self;
        known
            .next()
            .map(|(k, v)| (HeaderName::from(*k), v))
            .or_else(|| unknown.next().map(|(k, v)| (HeaderName::from(&**k), v)))
    }
}

impl IntoIterator for Headers {
    type Item = (HeaderName<'static>, HeaderValues);

    type IntoIter = IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Headers, KnownHeaderName};

    #[test]
    fn header_names_are_case_insensitive_for_access_but_retain_initial_case_in_headers() {
        let mut headers = Headers::new();
        headers.insert("my-Header-name", "initial-value");
        headers.insert("my-Header-NAME", "my-header-value");

        assert_eq!(headers.len(), 1);

        assert_eq!(
            headers.get_str("My-Header-Name").unwrap(),
            "my-header-value"
        );

        headers.append("mY-hEaDer-NaMe", "second-value");
        assert_eq!(
            headers.get_values("my-header-name").unwrap(),
            ["my-header-value", "second-value"].as_slice()
        );

        assert_eq!(
            headers.iter().next().unwrap().0.to_string(),
            "my-Header-name"
        );

        assert!(headers.remove("my-HEADER-name").is_some());
        assert!(headers.is_empty());
    }

    #[test]
    fn value_case_insensitive_comparison() {
        let mut headers = Headers::new();
        headers.insert(KnownHeaderName::Upgrade, "WebSocket");
        headers.insert(KnownHeaderName::Connection, "upgrade");

        assert!(headers.eq_ignore_ascii_case(KnownHeaderName::Upgrade, "websocket"));
        assert!(headers.eq_ignore_ascii_case(KnownHeaderName::Connection, "Upgrade"));
    }
}
