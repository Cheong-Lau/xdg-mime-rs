use memchr::memmem::Finder;
use nom::branch::alt;
use nom::bytes::complete::{is_a, tag, take, take_until};
use nom::character::complete::{char, line_ending};
use nom::combinator::{into, opt};
use nom::multi::{many0, many1};
use nom::number::complete::{be_u16, hex_u32};
use nom::sequence::{delimited, preceded, separated_pair, terminated};
use nom::IResult;
use nom::{ParseTo as _, Parser as _};
use std::fmt;
use std::fs::read;
use std::io;
use std::path::Path;
use std::vec::Vec;

use mime::Mime;

#[derive(Clone, Debug)]
struct MagicRule {
    indent: u32,
    start_offset: u32,
    finder: Finder<'static>,
    mask: Option<Box<[u8]>>,
    word_size: u32,
    range_length: u32,
}

// Since `Finder` doesn't impl PartialEq, need a manual implementation

impl PartialEq for MagicRule {
    fn eq(&self, other: &Self) -> bool {
        self.indent == other.indent
            && self.start_offset == other.start_offset
            && *self.value() == *other.value()
            && self.mask == other.mask
            && self.word_size == other.word_size
            && self.range_length == other.range_length
    }
}

fn masked_slices_are_equal(a: &[u8], b: &[u8], mask: &[u8]) -> bool {
    assert!(a.len() == b.len() && a.len() == mask.len());

    let masked_a = a.iter().zip(mask.iter()).map(|(x, m)| *x & *m);
    let masked_b = b.iter().zip(mask.iter()).map(|(x, m)| *x & *m);

    masked_a.eq(masked_b)
}

impl MagicRule {
    #[inline]
    fn value(&self) -> &[u8] {
        self.finder.needle()
    }

    #[inline]
    fn value_len(&self) -> usize {
        self.value().len()
    }

    fn matches_data(&self, data: &[u8]) -> bool {
        let mask = self.mask.as_deref();
        let value = self.value();
        let value_len = value.len();
        assert!(mask.map_or(true, |mask| mask.len() == value_len));

        let start = self.start_offset as usize;
        let range_length = self.range_length as usize;

        match mask {
            Some(mask) => {
                let mut data_windows = data.windows(value_len).skip(start).take(range_length);
                data_windows.any(|data_w| masked_slices_are_equal(data_w, self.value(), mask))
            }

            None => {
                let end = (start + value_len + range_length - 1).min(data.len());
                let start = start.min(end);
                self.finder.find(&data[start..end]).is_some()
            }
        }
    }

    fn extent(&self) -> usize {
        let value_len = self.value_len();
        let offset = self.start_offset as usize;
        let range_len = self.range_length as usize;

        value_len + offset + range_len
    }
}

// Indentation level, can be 0
fn indent_level(bytes: &[u8]) -> IResult<&[u8], u32> {
    is_a("0123456789>")
        .and_then(take_until(">"))
        .map(|s: &[u8]| s.parse_to().unwrap_or(0))
        .parse(bytes)
}

// Offset, can be 0
fn start_offset(bytes: &[u8]) -> IResult<&[u8], u32> {
    take_until("=")
        .map(|s: &[u8]| s.parse_to().unwrap_or(0))
        .parse(bytes)
}

// <word_size> = '~' (0 | 1 | 2 | 4)
fn word_size(bytes: &[u8]) -> IResult<&[u8], Option<u32>> {
    let alt_size = alt([char('0'), char('1'), char('2'), char('4')]).map_opt(|n| n.to_digit(10));
    let word_size = preceded(char('~'), alt_size);

    opt(word_size).parse(bytes)
}

// <range_length> = '+' <u32>
fn range_length(bytes: &[u8]) -> IResult<&[u8], Option<u32>> {
    opt(preceded(char('+'), hex_u32)).parse(bytes)
}

// magic_rule =
// [ <indent> ] '>' <start-offset> '=' <value_length> <value>
// [ '&' <mask> ] [ <word_size> ] [ <range_length> ]
// '\n'

fn finder(bytes: &[u8], length: u16) -> IResult<&[u8], Finder<'static>> {
    take(length)
        .map(|b| Finder::new(b).into_owned())
        .parse(bytes)
}

fn mask(bytes: &[u8], length: u16) -> IResult<&[u8], Option<Box<[u8]>>> {
    opt(preceded(char('&'), into(take(length)))).parse(bytes)
}

fn magic_rule(bytes: &[u8]) -> IResult<&[u8], MagicRule> {
    let (bytes, (indent, start_offset, value_length)) =
        (indent_level, start_offset, preceded(char('='), be_u16)).parse(bytes)?;

    let (bytes, finder) = finder(bytes, value_length)?;
    let (bytes, mask) = mask(bytes, value_length)?;

    let (bytes, (word_size, range_length)) =
        terminated((word_size, range_length), line_ending).parse(bytes)?;

    Ok((
        bytes,
        MagicRule {
            indent,
            start_offset,
            finder,
            mask,
            word_size: word_size.unwrap_or(1),
            range_length: range_length.unwrap_or(1),
        },
    ))
}

#[derive(Clone, PartialEq)]
pub struct MagicEntry {
    mime_type: Mime,
    priority: u32,
    rules: Box<[MagicRule]>,
}

impl fmt::Debug for MagicEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MIME type: {:?} (priority: {:?}):\nrules:\n{:?}",
            self.mime_type, self.priority, self.rules
        )
    }
}

impl MagicEntry {
    fn matches(&self, data: &[u8]) -> Option<(&Mime, u32)> {
        let mut current_level = 0;

        let mut iter = self.rules.iter().peekable();
        while let Some(rule) = iter.next() {
            // The rules are a flat list that represent a tree; the "indent"
            // is the depth of the rule in the tree.
            //
            // Check the rule at the current level
            if rule.indent == current_level && rule.matches_data(data) {
                // If the next rule has a lower level, or it's the last
                // rule, we found our match
                match iter.peek() {
                    Some(next) => {
                        if next.indent <= current_level {
                            return Some((&self.mime_type, self.priority));
                        }

                        // Otherwise, increase the level and check the
                        // next rule
                        current_level += 1;
                    }
                    None => {
                        // last rule
                        return Some((&self.mime_type, self.priority));
                    }
                }
            }
        }

        None
    }

    fn max_extents(&self) -> usize {
        self.rules.iter().map(MagicRule::extent).max().unwrap_or(0)
    }
}

fn priority(bytes: &[u8]) -> IResult<&[u8], u32> {
    take_until(":")
        .map(|s: &[u8]| s.parse_to().unwrap_or(0))
        .parse(bytes)
}

fn mime_type(bytes: &[u8]) -> IResult<&[u8], Mime> {
    take_until("]\n")
        .map_opt(|s: &[u8]| s.parse_to())
        .parse(bytes)
}

// magic_header =
// '[' <priority> ':' <mime_type> ']' '\n'
fn magic_header(bytes: &[u8]) -> IResult<&[u8], (u32, Mime)> {
    delimited(
        char('['),
        separated_pair(priority, char(':'), mime_type),
        tag("]\n"),
    )
    .parse(bytes)
}

// magic_entry =
// <magic_header>
// <magic_rule>+
fn magic_entry(bytes: &[u8]) -> IResult<&[u8], MagicEntry> {
    (magic_header, many1(magic_rule))
        .map(|(header, rules)| MagicEntry {
            priority: header.0,
            mime_type: header.1,
            rules: rules.into_boxed_slice(),
        })
        .parse(bytes)
}

fn from_u8_to_entries(bytes: &[u8]) -> IResult<&[u8], Vec<MagicEntry>> {
    preceded(tag("MIME-Magic\0\n"), many0(magic_entry)).parse(bytes)
}

pub fn lookup_data(entries: &[MagicEntry], data: &[u8]) -> Option<(Mime, u32)> {
    entries
        .iter()
        .find_map(|e| e.matches(data))
        .map(|v| (v.0.clone(), v.1))
}

pub fn max_extents(entries: &[MagicEntry]) -> usize {
    entries
        .iter()
        .map(MagicEntry::max_extents)
        .max()
        .unwrap_or(0)
}

pub fn read_magic_from_dir<P: AsRef<Path>>(dir: P) -> io::Result<Vec<MagicEntry>> {
    let magic_file = dir.as_ref().join("magic");
    read(magic_file).map(|magic_buf| {
        from_u8_to_entries(magic_buf.as_slice()).map_or_else(|_| Vec::new(), |v| v.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::HexDisplay;
    use nom::Offset;

    #[test]
    fn parse_magic_header() {
        let res = magic_header(b"[50:application/x-yaml]\n");

        match res {
            Ok((i, o)) => {
                assert_eq!(i.len(), 0);
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic rule");
            }
        }
    }

    #[test]
    fn parse_one_magic_rule() {
        let simple = include_bytes!("../test_files/parser/single_rule");
        println!("bytes:\n{}", &simple.to_hex(8));
        let simple_res = magic_rule(simple);

        match simple_res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, simple.offset(i)));
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic rule");
            }
        }

        let range = include_bytes!("../test_files/parser/rule_with_range");
        println!("bytes:\n{}", &range.to_hex(8));
        let range_res = magic_rule(range);

        match range_res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, range.offset(i)));
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic rule");
            }
        }

        let ws = include_bytes!("../test_files/parser/rule_with_ws");
        println!("bytes:\n{}", &ws.to_hex(8));
        let ws_res = magic_rule(ws);

        match ws_res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, ws.offset(i)));
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic rule");
            }
        }
    }

    #[test]
    fn parse_simple_magic_entry() {
        let data = include_bytes!("../test_files/parser/single_entry");
        println!("bytes:\n{}", &data.to_hex(8));
        let res = magic_entry(data);

        match res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, data.offset(i)));
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic entry");
            }
        }
    }

    #[test]
    fn parse_magic_entry() {
        let data = include_bytes!("../test_files/parser/many_rules");
        println!("bytes:\n{}", &data.to_hex(8));
        let res = magic_entry(data);

        match res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, data.offset(i)));
                println!("parsed:\n{o:?}");
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic entry");
            }
        }
    }

    #[test]
    fn parse_magic_file() {
        let data = include_bytes!("../test_files/mime/magic");
        let res = from_u8_to_entries(data);

        match res {
            Ok((i, o)) => {
                println!("remaining:\n{}", &i.to_hex_from(8, data.offset(i)));
                println!("parsed {} magic entries:\n{:#?}", o.len(), o);
            }
            Err(e) => {
                println!("invalid or incomplete: {e}");
                panic!("cannot parse magic file");
            }
        }
    }

    #[test]
    fn magic_rule_matches_data() {
        let rule = MagicRule {
            indent: 0,
            start_offset: 0,
            finder: Finder::new(b"hello"),
            mask: None,
            word_size: 1,
            range_length: 30,
        };

        assert!(rule.matches_data(b"hello world"));
        assert!(rule.matches_data(b"world hello"));
    }

    #[test]
    fn magic_rule_matches_data_with_start_offset() {
        let rule = MagicRule {
            indent: 0,
            start_offset: 1,
            finder: Finder::new(b"hello"),
            mask: None,
            word_size: 1,
            range_length: 30,
        };

        assert!(!rule.matches_data(b"hello world"));
        assert!(rule.matches_data(b"xhello world"));
        assert!(rule.matches_data(b"world hello"));
    }

    #[test]
    fn magic_rule_matches_data_with_range_length() {
        let rule = MagicRule {
            indent: 0,
            start_offset: 0,
            finder: Finder::new(b"hello"),
            mask: None,
            word_size: 1,
            range_length: 10,
        };

        assert!(rule.matches_data(b"hello world"));
        assert!(rule.matches_data(b"12345hello"));
        assert!(rule.matches_data(b"123456789hello"));
        assert!(!rule.matches_data(b"1234567890hello"));
        assert!(!rule.matches_data(b"too long a prefix for this to match hello"));
    }

    #[test]
    fn magic_rule_matches_data_with_start_offset_and_range_length() {
        let rule = MagicRule {
            indent: 0,
            start_offset: 1,
            finder: Finder::new(b"hello"),
            mask: None,
            word_size: 1,
            range_length: 3,
        };

        assert!(!rule.matches_data(b"hello world"));
        assert!(rule.matches_data(b"1hello world"));
        assert!(rule.matches_data(b"12hello world"));
        assert!(rule.matches_data(b"123hello world"));
        assert!(!rule.matches_data(b"1234hello world"));
    }

    #[test]
    fn magic_rule_matches_data_with_mask() {
        let rule = MagicRule {
            indent: 0,
            start_offset: 0,
            finder: Finder::new(b"hello"),
            mask: Some(Box::from([!0x20; 5])),
            word_size: 1,
            range_length: 30,
        };

        assert!(rule.matches_data(b"HeLlo world"));
        assert!(rule.matches_data(b"world HeLlo"));
        assert!(rule.matches_data(b"12345heLLO"));
        assert!(!rule.matches_data(b"HuLLO WORLD"));
    }
}
