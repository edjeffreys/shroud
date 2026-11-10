//! Cells name their OSC 8 link by id so they stay `Copy`. Ids are never reused, so a
//! stale id left in an old frame can't be mistaken for a different link.

use std::collections::{HashMap, HashSet};

pub type LinkId = u32;

pub const NO_LINK: LinkId = 0;

// OSC 8's suggested limit; longer is more likely abuse than a real URI.
const MAX_URI: usize = 2083;

#[derive(Debug, Default)]
pub struct Links {
    ids: HashMap<String, LinkId>,
    uris: HashMap<LinkId, String>,
    last: LinkId,
}

impl Links {
    pub fn intern(&mut self, uri: &str) -> LinkId {
        if uri.is_empty() || uri.len() > MAX_URI || uri.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return NO_LINK;
        }
        if let Some(&id) = self.ids.get(uri) {
            return id;
        }
        self.last += 1;
        self.ids.insert(uri.to_string(), self.last);
        self.uris.insert(self.last, uri.to_string());
        self.last
    }

    pub fn uri(&self, id: LinkId) -> Option<&str> {
        self.uris.get(&id).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.uris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.uris.is_empty()
    }

    pub fn retain(&mut self, live: &HashSet<LinkId>) {
        self.uris.retain(|id, _| live.contains(id));
        self.ids.retain(|_, id| live.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_stable_and_ids_are_not_reused() {
        let mut l = Links::default();
        let a = l.intern("https://a");
        assert_eq!(l.intern("https://a"), a);
        l.retain(&HashSet::new());
        assert!(l.is_empty());
        let b = l.intern("https://a");
        assert_ne!(a, b);
        assert_eq!(l.uri(b), Some("https://a"));
        assert_eq!(l.uri(a), None);
    }

    #[test]
    fn unsafe_uris_get_no_link() {
        let mut l = Links::default();
        assert_eq!(l.intern("https://x\x1b\\evil"), NO_LINK);
        assert_eq!(l.intern(""), NO_LINK);
        assert_eq!(l.intern(&"a".repeat(MAX_URI + 1)), NO_LINK);
    }
}
