use std::fmt;
use std::io;
use std::path::Path;
use std::str::FromStr;

use mime::Mime;

#[derive(Clone, PartialEq, Eq)]
pub struct Alias {
    pub alias: Mime,
    pub mime_type: Mime,
}

impl fmt::Debug for Alias {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Alias {} {}", self.alias, self.mime_type)
    }
}

impl std::cmp::PartialOrd for Alias {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Ord for Alias {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.alias.cmp(&other.alias)
    }
}

impl Alias {
    pub fn new(alias: &Mime, mime_type: &Mime) -> Alias {
        Alias {
            alias: alias.clone(),
            mime_type: mime_type.clone(),
        }
    }

    pub fn from_string(s: &str) -> Option<Alias> {
        let mut chunks = s.split_whitespace();
        let alias = chunks.next().and_then(|s| Mime::from_str(s).ok())?;
        let mime_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;

        // Consume the leftovers, if any
        if chunks.next().is_some() {
            return None;
        }

        Some(Alias { alias, mime_type })
    }

    pub fn is_equivalent(&self, other: &Alias) -> bool {
        self.alias == other.alias
    }
}

#[derive(Default)]
pub struct AliasesList {
    aliases: Vec<Alias>,
}

impl AliasesList {
    pub fn new() -> AliasesList {
        AliasesList::default()
    }

    pub fn sort(&mut self) {
        self.aliases.sort();
    }

    pub fn unalias_mime_type(&self, mime_type: &Mime) -> Option<Mime> {
        self.aliases
            .iter()
            .find(|&a| a.alias == *mime_type)
            .map(|a| a.mime_type.clone())
    }

    pub fn clear(&mut self) {
        self.aliases.clear();
    }

    pub fn add_aliases_from_dir<P: AsRef<Path>>(&mut self, dir: P) -> io::Result<()> {
        let alias_dir = dir.as_ref().join("aliases");
        crate::extend_from_path(self, &alias_dir, Alias::from_string)
    }
}

impl Extend<Alias> for AliasesList {
    fn extend<T: IntoIterator<Item = Alias>>(&mut self, iter: T) {
        self.aliases.extend(iter);
        self.aliases.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_alias() {
        assert!(Alias::new(
            &"application/foo".parse().unwrap(),
            &"application/foo".parse().unwrap()
        )
        .is_equivalent(&Alias::new(
            &"application/foo".parse().unwrap(),
            &"application/x-foo".parse().unwrap()
        )),);
    }

    #[test]
    fn from_str() {
        assert_eq!(
            Alias::from_string("application/x-foo application/foo").unwrap(),
            Alias::new(
                &"application/x-foo".parse().unwrap(),
                &"application/foo".parse().unwrap(),
            )
        );
    }

    #[test]
    fn extra_tokens_yield_error() {
        assert!(Alias::from_string("one/foo two/foo three/foo").is_none());
    }
}
