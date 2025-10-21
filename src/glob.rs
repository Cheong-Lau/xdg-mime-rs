use std::cmp::Reverse;
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::str::FromStr;

use glob::Pattern;
use mime::Mime;
use unicase::UniCase;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum GlobType {
    Literal(Box<str>),
    Simple(Box<str>),
    Full(Pattern),
}

impl fmt::Debug for GlobType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GlobType::Literal(name) => write!(f, "Literal '{name}'"),
            GlobType::Simple(pattern) => write!(f, "Simple glob '*{pattern}'"),
            GlobType::Full(pattern) => write!(f, "Full glob '{pattern}'"),
        }
    }
}

impl GlobType {
    fn as_str(&self) -> &str {
        match self {
            GlobType::Literal(str) | GlobType::Simple(str) => str,
            GlobType::Full(pattern) => pattern.as_str(),
        }
    }
}

fn determine_type(glob: &str) -> GlobType {
    let mut maybe_simple = false;

    for (idx, ch) in glob.bytes().enumerate() {
        if idx == 0 && ch == b'*' {
            maybe_simple = true;
        } else if ch == b'\\' || ch == b'[' || ch == b'*' || ch == b'?' {
            return GlobType::Full(Pattern::new(glob).unwrap());
        }
    }

    if maybe_simple {
        GlobType::Simple(glob[1..].into())
    } else {
        GlobType::Literal(glob.into())
    }
}

#[derive(Clone)]
pub struct Glob {
    glob: GlobType,
    weight: i32,
    case_sensitive: bool,
    mime_type: Mime,
}

impl PartialEq for Glob {
    fn eq(&self, other: &Glob) -> bool {
        self.glob == other.glob && self.mime_type == other.mime_type
    }
}

impl Eq for Glob {}

impl Hash for Glob {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.glob.hash(h);
        self.mime_type.hash(h);
    }
}

impl fmt::Debug for Glob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Glob: {:?} {:?} (weight: {}, cs: {})",
            self.glob, self.mime_type, self.weight, self.case_sensitive
        )
    }
}

impl Glob {
    pub fn simple(mime_type: Mime, glob: &str) -> Glob {
        Glob::with_weight(mime_type, glob, 50)
    }

    pub fn with_weight(mime_type: Mime, glob: &str, weight: i32) -> Glob {
        Glob::new(mime_type, glob, weight, false)
    }

    pub fn new(mime_type: Mime, glob: &str, weight: i32, case_sensitive: bool) -> Glob {
        Glob {
            mime_type,
            glob: determine_type(glob),
            weight,
            case_sensitive,
        }
    }

    pub fn from_v1_string(s: &str) -> Option<Glob> {
        let mut chunks = s.split(':');
        let mime_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;
        let glob = chunks.next().filter(|&s| !s.is_empty())?;

        // The globs file is not extensible, so consume any
        // leftover tokens
        if chunks.next().is_some() {
            return None;
        }

        Some(Glob::simple(mime_type, glob))
    }

    pub fn from_v2_string(s: &str) -> Option<Glob> {
        let mut chunks = s.split(':');

        let weight = chunks
            .next()
            .and_then(|v| i32::from_str(v).ok())
            .filter(|n| *n >= 0)?;

        let mime_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;
        let glob = chunks.next()?;

        let mut case_sensitive = false;
        if let Some(flags) = chunks.next() {
            // Allow for extra flags
            if flags.split(',').any(|x| x == "cs") {
                case_sensitive = true;
            }
        }

        // Ignore any other token, for extensibility:
        //
        // https://specifications.freedesktop.org/shared-mime-info-spec/shared-mime-info-spec-latest.html#idm46152099256048

        Some(Glob::new(mime_type, glob, weight, case_sensitive))
    }

    fn compare(&self, file_name: &str) -> bool {
        match &self.glob {
            GlobType::Literal(s) => {
                let a = UniCase::new(s);
                let b = UniCase::new(file_name);

                return a == b;
            }
            GlobType::Simple(s) => {
                if file_name.ends_with(s.as_ref()) {
                    return true;
                }

                if !self.case_sensitive {
                    let lc_file_name = file_name.to_lowercase();
                    if lc_file_name.ends_with(s.as_ref()) {
                        return true;
                    }
                }
            }
            GlobType::Full(p) => {
                return p.matches(file_name);
            }
        }

        false
    }
}

pub fn read_globs_v1_from_file<P: AsRef<Path>>(file_name: P) -> Option<Vec<Glob>> {
    let Ok(f) = File::open(file_name) else {
        return None;
    };

    let mut res = Vec::new();
    let file = BufReader::new(&f);
    for line in file.lines() {
        if line.is_err() {
            return None;
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(glob) = Glob::from_v1_string(&line) {
            res.push(glob);
        }
    }

    Some(res)
}

pub fn read_globs_v2_from_file<P: AsRef<Path>>(file_name: P) -> Option<Vec<Glob>> {
    let Ok(f) = File::open(file_name) else {
        return None;
    };

    let mut res = Vec::new();
    let file = BufReader::new(&f);
    for line in file.lines() {
        if line.is_err() {
            return None;
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(glob) = Glob::from_v2_string(&line) {
            res.push(glob);
        }
    }

    Some(res)
}

pub fn read_globs_from_dir<P: AsRef<Path>>(dir: P) -> Vec<Glob> {
    let mut globs_file = dir.as_ref().join("globs2");

    read_globs_v2_from_file(&globs_file).unwrap_or_else(|| {
        globs_file.pop();
        globs_file.push("globs");
        read_globs_v1_from_file(globs_file).unwrap_or_default()
    })
}

#[derive(Default)]
pub struct GlobMap {
    globs: HashSet<Glob>,
}

impl GlobMap {
    pub fn new() -> GlobMap {
        GlobMap::default()
    }

    pub fn add_glob(&mut self, glob: Glob) {
        self.globs.insert(glob);
    }

    pub fn add_globs(&mut self, globs: impl IntoIterator<Item = Glob>) {
        self.globs.extend(globs);
    }

    pub fn lookup_mime_type_for_file_name(&self, file_name: &str) -> Option<Vec<Mime>> {
        let mut matching_globs: Vec<_> = self
            .globs
            .iter()
            .filter(|&glob| glob.compare(file_name))
            .collect();

        // Sort in descending order by weight
        matching_globs.sort_unstable_by_key(|&glob| Reverse(glob.weight));

        let biggest_weight = matching_globs.first()?.weight;

        // "Keep only globs with the biggest weight."
        // -- shared-mime-info, "Recommended checking order"
        let matching_globs = matching_globs
            .into_iter()
            .filter(|&glob| glob.weight == biggest_weight);

        // Needs to be after filtering for biggest weight
        // in case it changes which glob is the longest.
        let biggest_glob_length = matching_globs
            .clone()
            .map(|glob| glob.glob.as_str().len())
            .max()?;

        // "If the patterns are different, keep only the globs
        // with the longest pattern, as previously discussed."
        // -- shared-mime-info, "Recommended checking order"
        let res = matching_globs
            .filter(|&glob| glob.glob.as_str().len() == biggest_glob_length)
            .map(|glob| glob.mime_type.clone())
            .collect();

        Some(res)
    }

    pub fn clear(&mut self) {
        self.globs.clear();
    }
}

impl fmt::Debug for GlobMap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Globs:\n")?;
        self.globs.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_type() {
        assert_eq!(determine_type("*.gif"), GlobType::Simple(".gif".into()));
        assert_eq!(
            determine_type("Foo*.gif"),
            GlobType::Full(Pattern::new("Foo*.gif").unwrap())
        );
        assert_eq!(
            determine_type("*[4].gif"),
            GlobType::Full(Pattern::new("*[4].gif").unwrap())
        );
        assert_eq!(
            determine_type("Makefile"),
            GlobType::Literal("Makefile".into())
        );
        assert_eq!(
            determine_type("sldkfjvlsdf\\\\slkdjf"),
            GlobType::Full(Pattern::new("sldkfjvlsdf\\\\slkdjf").unwrap())
        );
        assert_eq!(
            determine_type("tree.[ch]"),
            GlobType::Full(Pattern::new("tree.[ch]").unwrap())
        );
    }

    #[test]
    fn glob_v1_string() {
        assert_eq!(
            Glob::from_v1_string("text/rust:*.rs"),
            Some(Glob::simple("text/rust".parse().unwrap(), "*.rs"))
        );
        assert_eq!(
            Glob::from_v1_string("text/rust:*.rs"),
            Some(Glob::new("text/rust".parse().unwrap(), "*.rs", 50, false))
        );

        assert_eq!(Glob::from_v1_string(""), None);
        assert_eq!(Glob::from_v1_string("foo"), None);
        assert_eq!(Glob::from_v1_string("foo:"), None);
        assert_eq!(Glob::from_v1_string(":bar"), None);
        assert_eq!(Glob::from_v1_string(":"), None);
        assert_eq!(Glob::from_v1_string("foo:bar:baz"), None);
    }

    #[test]
    fn glob_v2_string() {
        assert_eq!(
            Glob::from_v2_string("80:text/rust:*.rs"),
            Some(Glob::with_weight("text/rust".parse().unwrap(), "*.rs", 80))
        );
        assert_eq!(
            Glob::from_v2_string("80:text/rust:*.rs"),
            Some(Glob::new("text/rust".parse().unwrap(), "*.rs", 80, false))
        );
        assert_eq!(
            Glob::from_v2_string("50:text/x-c++src:*.C:cs"),
            Some(Glob::new("text/x-c++src".parse().unwrap(), "*.C", 50, true))
        );

        assert_eq!(Glob::from_v2_string(""), None);
        assert_eq!(Glob::from_v2_string("foo"), None);
        assert_eq!(Glob::from_v2_string("foo:"), None);
        assert_eq!(Glob::from_v2_string(":bar"), None);
        assert_eq!(Glob::from_v2_string(":"), None);
        assert_eq!(Glob::from_v2_string("foo:bar:baz"), None);
        assert_eq!(Glob::from_v2_string("foo:bar:baz:blah"), None);

        assert_eq!(
            Glob::from_v2_string("50:text/x-c++src:*.C:cs,newflag:newfeature:somethingelse"),
            Some(Glob::new("text/x-c++src".parse().unwrap(), "*.C", 50, true))
        );
    }

    #[test]
    fn compare() {
        // Literal
        let copying = Glob::new("text/x-copying".parse().unwrap(), "copying", 50, false);
        assert!(copying.compare("COPYING"));

        // Simple, case-insensitive
        let c_src = Glob::new("text/x-csrc".parse().unwrap(), "*.c", 50, false);
        assert!(c_src.compare("foo.c"));
        assert!(c_src.compare("FOO.C"));

        // Simple, case-sensitive
        let cplusplus_src = Glob::new("text/x-c++src".parse().unwrap(), "*.C", 50, true);
        assert!(cplusplus_src.compare("foo.C"));
        assert!(!cplusplus_src.compare("foo.c"));
        assert!(!cplusplus_src.compare("foo.h"));

        // Full
        let video_x_anim = Glob::new("video/x-anim".parse().unwrap(), "*.anim[1-9j]", 50, false);
        assert!(!video_x_anim.compare("foo.anim0"));
        assert!(video_x_anim.compare("foo.anim8"));
        assert!(!video_x_anim.compare("foo.animk"));
        assert!(video_x_anim.compare("foo.animj"));
    }
}
