use std::fmt;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
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

    pub fn add_aliases(&mut self, aliases: impl IntoIterator<Item = Alias>) {
        self.aliases.extend(aliases);
        self.sort();
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
}

pub fn read_aliases_from_file<P: AsRef<Path>>(file_name: P) -> Vec<Alias> {
    let mut res = Vec::new();

    let Ok(f) = File::open(file_name) else {
        return res;
    };

    let file = BufReader::new(&f);
    for line in file.lines() {
        if line.is_err() {
            return res; // FIXME: return error instead
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(alias) = Alias::from_string(&line) {
            res.push(alias);
        }
    }

    res
}

pub fn read_aliases_from_dir<P: AsRef<Path>>(dir: P) -> Vec<Alias> {
    let alias_file = dir.as_ref().join("aliases");

    read_aliases_from_file(alias_file)
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
