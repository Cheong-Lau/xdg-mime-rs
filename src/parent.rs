use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::str::FromStr;

use mime::Mime;

#[derive(Clone, PartialEq, Eq)]
pub struct Subclass {
    mime_type: Mime,
    parent_type: Mime,
}

impl Subclass {
    pub fn new(mime_type: &Mime, parent_type: &Mime) -> Subclass {
        Subclass {
            mime_type: mime_type.clone(),
            parent_type: parent_type.clone(),
        }
    }

    fn from_string(s: &str) -> Option<Subclass> {
        let mut chunks = s.split_whitespace();
        let mime_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;
        let parent_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;

        // Consume the leftovers, if any
        if chunks.next().is_some() {
            return None;
        }

        Some(Subclass {
            mime_type,
            parent_type,
        })
    }
}

impl fmt::Debug for Subclass {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Subclass {} {}", self.parent_type, self.mime_type)
    }
}

#[derive(Default)]
pub struct ParentsMap {
    parents: FxHashMap<Mime, FxHashSet<Mime>>,
}

impl ParentsMap {
    pub fn new() -> ParentsMap {
        ParentsMap::default()
    }

    fn add_subclass(&mut self, subclass: Subclass) {
        let v = self.parents.entry(subclass.mime_type).or_default();
        v.insert(subclass.parent_type);
    }

    pub fn lookup(&self, mime_type: &Mime) -> Option<&FxHashSet<Mime>> {
        self.parents.get(mime_type)
    }

    pub fn clear(&mut self) {
        self.parents.clear();
    }

    pub fn add_subclasses_from_dir<P: AsRef<Path>>(&mut self, dir: P) -> io::Result<()> {
        let subclasses_dir = dir.as_ref().join("subclasses");
        crate::extend_from_path(self, &subclasses_dir, Subclass::from_string)
    }
}

impl Extend<Subclass> for ParentsMap {
    fn extend<T: IntoIterator<Item = Subclass>>(&mut self, iter: T) {
        let iter = iter.into_iter();
        self.parents.reserve(iter.size_hint().0);

        for subclass in iter {
            self.add_subclass(subclass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str() {
        assert_eq!(
            Subclass::from_string("message/partial text/plain").unwrap(),
            Subclass::new(
                &"message/partial".parse().unwrap(),
                &"text/plain".parse().unwrap()
            )
        );
    }

    #[test]
    fn parent_map() {
        let mut pm = ParentsMap::new();

        pm.add_subclass(Subclass::new(
            &"message/partial".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        pm.add_subclass(Subclass::new(
            &"text/rfc822-headers".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));

        assert_eq!(
            pm.lookup(&"message/partial".parse().unwrap()),
            Some(
                &[Mime::from_str("text/plain").unwrap()]
                    .into_iter()
                    .collect()
            ),
        );
    }

    #[test]
    fn extra_tokens_yield_error() {
        assert!(Subclass::from_string("one/foo two/foo three/foo").is_none());
    }
}
