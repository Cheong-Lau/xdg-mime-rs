use std::fmt;
use std::io;
use std::path::Path;
use std::str::FromStr;

use mime::Mime;

#[derive(Clone, PartialEq, Eq)]
pub struct Icon {
    icon_name: Box<str>,
    mime_type: Mime,
}

impl fmt::Debug for Icon {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Icon for {}: {}", self.mime_type, self.icon_name)
    }
}

impl Icon {
    pub fn new(icon_name: &str, mime_type: &Mime) -> Icon {
        Icon {
            icon_name: icon_name.into(),
            mime_type: mime_type.clone(),
        }
    }

    pub fn from_string(s: &str) -> Option<Icon> {
        let mut chunks = s.split(':');
        let mime_type = chunks.next().and_then(|s| Mime::from_str(s).ok())?;
        let icon_name = chunks.next().filter(|&s| !s.is_empty())?;

        // Consume the leftovers, if any
        if chunks.next().is_some() {
            return None;
        }

        Some(Icon {
            icon_name: icon_name.into(),
            mime_type,
        })
    }
}

pub fn add_icons_from_dir<P: AsRef<Path>>(
    dir: P,
    generic: bool,
    vector: &mut Vec<Icon>,
) -> io::Result<()> {
    let icons_file = if generic {
        dir.as_ref().join("generic-icons")
    } else {
        dir.as_ref().join("icons")
    };

    crate::extend_from_path(vector, &icons_file, Icon::from_string)?;

    vector.sort_by(|a, b| a.mime_type.cmp(&b.mime_type));
    Ok(())
}

pub fn find_icon<'a>(icons: &'a [Icon], mime_type: &Mime) -> Option<&'a str> {
    icons
        .iter()
        .find(|&icon| icon.mime_type == *mime_type)
        .map(|icon| icon.icon_name.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str() {
        assert_eq!(
            Icon::from_string("application/rss+xml:text-html").unwrap(),
            Icon::new("text-html", &"application/rss+xml".parse().unwrap())
        );
    }

    #[test]
    fn from_str_catches_syntax_error() {
        assert!(Icon::from_string("one:two:three").is_none());
        assert!(Icon::from_string(":").is_none());
        assert!(Icon::from_string("one:").is_none());
        assert!(Icon::from_string(":two").is_none());
        assert!(Icon::from_string("").is_none());
    }
}
