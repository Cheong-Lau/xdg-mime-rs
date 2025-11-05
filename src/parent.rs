use rustc_hash::FxHashMap;
use std::fmt;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
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
    parents: FxHashMap<Mime, Vec<Mime>>,
}

impl ParentsMap {
    pub fn new() -> ParentsMap {
        ParentsMap::default()
    }

    fn add_subclass(&mut self, subclass: Subclass) {
        let v = self.parents.entry(subclass.mime_type).or_default();
        if !v.contains(&subclass.parent_type) {
            v.push(subclass.parent_type);
        }
    }

    pub fn add_subclasses(&mut self, subclasses: impl IntoIterator<Item = Subclass>) {
        for s in subclasses {
            self.add_subclass(s);
        }
    }

    pub fn lookup(&self, mime_type: &Mime) -> Option<&Vec<Mime>> {
        self.parents.get(mime_type)
    }

    pub fn clear(&mut self) {
        self.parents.clear();
    }
}

pub fn read_subclasses_from_file<P: AsRef<Path>>(file_name: P) -> Vec<Subclass> {
    let Ok(f) = File::open(file_name) else {
        return Vec::new();
    };

    let mut res = Vec::new();
    let file = BufReader::new(&f);
    for line in file.lines() {
        if line.is_err() {
            return res; // FIXME: return error instead
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(subclass) = Subclass::from_string(&line) {
            res.push(subclass);
        }
    }

    res
}

pub fn read_subclasses_from_dir<P: AsRef<Path>>(dir: P) -> Vec<Subclass> {
    let subclasses_file = dir.as_ref().join("subclasses");

    read_subclasses_from_file(subclasses_file)
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
            Some(&vec![Mime::from_str("text/plain").unwrap()]),
        );
    }

    #[test]
    fn extra_tokens_yield_error() {
        assert!(Subclass::from_string("one/foo two/foo three/foo").is_none());
    }
}
