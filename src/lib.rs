#![cfg(any(unix, target_os = "redox"))]
#![doc(html_root_url = "https://docs.rs/xdg_mime/0.4.0")]
#![allow(dead_code)]

//! `xdg_mime` allows to look up the MIME type associated to a file name
//! or to the contents of a file, using the [Freedesktop.org Shared MIME
//! database specification][xdg-mime].
//!
//! Alongside the MIME type, the shared MIME database contains other ancillary
//! information, like the icon associated to the MIME type; the aliases for
//! a given MIME type; and the various sub-classes of a MIME type.
//!
//! [xdg-mime]: https://specifications.freedesktop.org/shared-mime-info-spec/shared-mime-info-spec-latest.html
//!
//! ## Loading the Shared MIME database
//!
//! The [`SharedMimeInfo`](struct.SharedMimeInfo.html) type will automatically
//! load all the instances of shared MIME databases available in the following
//! directories, in this specified order:
//!
//!  - `$XDG_DATA_HOME/mime`
//!    - if `XDG_DATA_HOME` is unset, this corresponds to `$HOME/.local/share/mime`
//!  - `$XDG_DATA_DIRS/mime`
//!    - if `XDG_DATA_DIRS` is unset, this corresponds to `/usr/local/share/mime`
//!      and `/usr/share/mime`
//!
//! For more information on the `XDG_DATA_HOME` and `XDG_DATA_DIRS` environment
//! variables, see the [XDG base directory specification][xdg-basedir].
//!
//! [xdg-basedir]: https://specifications.freedesktop.org/basedir-spec/latest/
//!
//! The MIME data in each directory will be coalesced into a single database.
//!
//! ## Retrieving the MIME type of a file
//!
//! If you want to know the MIME type of a file, you typically have two
//! options at your disposal:
//!
//!  - guess from the file name, using the [`get_mime_types_from_file_name`]
//!    method
//!  - use an appropriately sized chunk of the file contents and
//!    perform "content sniffing", using the [`get_mime_type_for_data`] method
//!
//! The former step does not come with performance penalties, or even requires
//! the file to exist in the first place, but it may return a list of potential
//! matches; the latter can be an arbitrarily expensive operation to perform,
//! but its result is going to be certain. It is recommended to always guess the
//! MIME type from the file name first, and only use content sniffing lazily and,
//! possibly, asynchronously.
//!
//! [`get_mime_types_from_file_name`]: struct.SharedMimeInfo.html#method.get_mime_types_from_file_name
//! [`get_mime_type_for_data`]: struct.SharedMimeInfo.html#method.get_mime_type_for_data
//!
//! ## Guessing the MIME type
//!
//! If you have access to a file name or its contents, it's possible to use
//! the [`guess_mime_type`] method to create a [`GuessBuilder`] instance, and
//! populate it with the file name, its contents, or the full path to the file;
//! then, call the [`guess`] method to guess the MIME type depending on the
//! available information.
//!
//! [`GuessBuilder`]: struct.GuessBuilder.html
//! [`guess_mime_type`]: struct.SharedMimeInfo.html#method.guess_mime_type
//! [`guess`]: struct.GuessBuilder.html#method.guess

use mime::Mime;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

extern crate nom;

mod alias;
mod glob;
mod icon;
mod magic;
mod parent;

#[derive(Clone, PartialEq)]
struct MimeDirectory {
    path: Box<Path>,
    mtime: SystemTime,
}

/// The shared MIME info database.
pub struct SharedMimeInfo {
    aliases: alias::AliasesList,
    parents: parent::ParentsMap,
    icons: Vec<icon::Icon>,
    generic_icons: Vec<icon::Icon>,
    globs: glob::GlobMap,
    magic: Vec<magic::MagicEntry>,
    mime_dirs: Vec<MimeDirectory>,
}

/// A builder type to specify the parameters for guessing a MIME type.
///
/// Each instance of `GuessBuilder` is tied to the lifetime of the
/// [`SharedMimeInfo`] instance that created it.
///
/// The `GuessBuilder` returned by the [`guess_mime_type`] method is
/// empty, and will always return a `mime::APPLICATION_OCTET_STREAM`
/// guess.
///
/// You can use the builder methods to specify the file name, the data,
/// or both, to be used to guess the MIME type:
///
/// ```rust
/// # use std::error::Error;
/// # use mime::Mime;
/// #
/// # fn main() -> Result<(), Box<dyn Error>> {
/// # let mime_db = xdg_mime::SharedMimeInfo::new();
/// // let mime_db = ...
/// let mut guess_builder = mime_db.guess_mime_type();
/// let guess = guess_builder.file_name("foo.png").guess();
/// assert_eq!(*guess.mime_type(), "image/png");
/// #
/// # Ok(())
/// # }
/// ```
///
/// The guessed MIME type can have a degree of uncertainty; for instance,
/// if you only set the [`file_name`] there can be multiple matching MIME
/// types to choose from. Alternatively, if you only set the [`data`], the
/// content might not match any existing rule. Even in the case of setting
/// both the file name and the data the match can be uncertain. This
/// information is preserved by the [`Guess`] type, and can be retrieved
/// using the [`uncertain`] method.
///
/// [`SharedMimeInfo`]: struct.SharedMimeInfo.html
/// [`guess_mime_type`]: struct.SharedMimeInfo.html#method.guess_mime_type
/// [`file_name`]: #method.file_name
/// [`data`]: #method.data
/// [`Guess`]: struct.Guess.html
/// [`uncertain`]: struct.Guess.html#method.uncertain
pub struct GuessBuilder<'a> {
    db: &'a SharedMimeInfo,
    file_name: Option<Box<str>>,
    data: Vec<u8>,
    metadata: Option<fs::Metadata>,
    path: Option<Box<Path>>,
    zero_size: bool,
}

/// The result of the [`guess`] method of [`GuessBuilder`].
///
/// [`guess`]: struct.GuessBuilder.html#method.guess
/// [`GuessBuilder`]: struct.GuessBuilder.html
pub struct Guess {
    mime: mime::Mime,
    uncertain: bool,
}

impl GuessBuilder<'_> {
    /// Sets the file name to be used to guess its MIME type.
    ///
    /// If you have a full path, you should extract the last component,
    /// for instance using the [`Path::file_name()`][path_file_name]
    /// method.
    ///
    /// [path_file_name]: https://doc.rust-lang.org/std/path/struct.Path.html#method.file_name
    pub fn file_name(&mut self, name: &str) -> &mut Self {
        self.file_name = Some(name.into());

        self
    }

    /// Sets the data for which you want to guess the MIME type.
    pub fn data(&mut self, data: &[u8]) -> &mut Self {
        // If we have enough data, just copy the largest chunk
        // necessary to match any rule in the magic entries
        let max_data_size = magic::max_extents(&self.db.magic);
        if data.len() > max_data_size {
            self.data.extend(&data[..max_data_size]);
        } else {
            self.data.extend(data);
        }

        self
    }

    /// Sets the metadata of the file for which you want to get the MIME type.
    ///
    /// The metadata can be used to match an existing file or path, for instance:
    ///
    /// ```rust
    /// # use std::error::Error;
    /// use std::fs;
    /// use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// # let metadata = fs::metadata("src/lib.rs")?;
    /// // let metadata = fs::metadata("/path/to/lib.rs")?;
    /// let mut guess_builder = mime_db.guess_mime_type();
    /// let guess = guess_builder
    ///     .file_name("lib.rs")
    ///     .metadata(metadata)
    ///     .guess();
    /// assert_eq!(*guess.mime_type(), "text/rust");
    /// #
    /// # Ok(())
    /// # }
    /// ```
    pub const fn metadata(&mut self, metadata: fs::Metadata) -> &mut Self {
        self.metadata = Some(metadata);

        self
    }

    /// Sets the path of the file for which you want to get the MIME type.
    ///
    /// The `path` will be used by the [`guess`] method to extract the
    /// file name, metadata, and contents, unless you called the [`file_name`],
    /// [`metadata`], and [`data`] methods, respectively.
    ///
    /// ```rust
    /// # use std::error::Error;
    /// use std::fs;
    /// use std::str::FromStr;
    /// use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let mut guess_builder = mime_db.guess_mime_type();
    /// let guess = guess_builder
    ///     .path("src")
    ///     .guess();
    /// assert_eq!(*guess.mime_type(), "inode/directory");
    /// #
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`guess`]: #method.guess
    /// [`file_name`]: #method.file_name
    /// [`metadata`]: #method.metadata
    /// [`data`]: #method.data
    pub fn path<P: AsRef<Path>>(&mut self, path: P) -> &mut Self {
        self.path = Some(path.as_ref().into());

        self
    }

    /// Sets whether or not the [`guess`] method will return `application/x-zerosize` for empty files.
    ///
    /// If `allow` is set to `true`, using [`guess`] to guess the MIME types of empty files will result in `application/x-zerosize`.
    /// Otherwise, the check for empty files will be disabled and other methods will be used to determine the MIME type.
    /// Does not affect the results of non-empty files.
    ///
    /// Defaults to true.
    ///
    /// [`guess`]: #method.guess
    pub const fn zero_size(&mut self, allow: bool) -> &mut Self {
        self.zero_size = allow;

        self
    }

    /// Guesses the MIME type using the data set on the builder. The result is
    /// a [`Guess`] instance that contains both the guessed MIME type, and whether
    /// the result of the guess is certain.
    ///
    /// [`Guess`]: struct.Guess.html
    pub fn guess(&mut self) -> Guess {
        if let Some(path) = &self.path {
            // Fill out the metadata
            if self.metadata.is_none() {
                self.metadata = fs::metadata(path).ok();
            }

            // Set the file name
            if self.file_name.is_none() {
                self.file_name = path.file_name().and_then(OsStr::to_str).map(Box::from);
            }
        }

        if let Some(metadata) = &self.metadata {
            let file_type = metadata.file_type();

            // Special type for directories
            if file_type.is_dir() {
                return Guess {
                    mime: "inode/directory".parse().unwrap(),
                    uncertain: true,
                };
            }

            // Special type for symbolic links
            if file_type.is_symlink() {
                return Guess {
                    mime: "inode/symlink".parse().unwrap(),
                    uncertain: true,
                };
            }

            // Special type for empty files
            if self.zero_size && metadata.len() == 0 {
                return Guess {
                    mime: "application/x-zerosize".parse().unwrap(),
                    uncertain: true,
                };
            }
        }

        let name_mime_types_opt = if let Some(file_name) = self.file_name.as_ref() {
            Some(
                if let Some(name_mime_types) =
                    self.db.globs.lookup_mime_type_for_file_name(file_name)
                {
                    // Try to return early if the file name matches with no conflicts
                    match <[_; 1]>::try_from(name_mime_types) {
                        Ok([mime]) => return Guess::new_certain(mime), // File name match, and no conflicts
                        Err(name_mime_types) => Some(name_mime_types), // File name matches with conflicts
                    }
                } else {
                    None // No file name match
                },
            )
        } else {
            None // No file name
        };

        if let Some(path) = &self.path {
            fn load_data_chunk<P: AsRef<Path>>(path: P, chunk_size: usize) -> Option<Vec<u8>> {
                if chunk_size == 0 {
                    return None;
                }

                let mut f = fs::File::open(&path).ok()?;
                let mut buf = vec![0u8; chunk_size];
                f.read_exact(&mut buf).ok()?;

                Some(buf)
            }

            // Load the minimum amount of data necessary for a match
            if self.data.is_empty() {
                let mut max_data_size = magic::max_extents(&self.db.magic);

                if let Some(metadata) = &self.metadata {
                    max_data_size = max_data_size.min(metadata.len() as usize);
                }

                if let Some(data) = load_data_chunk(path, max_data_size) {
                    self.data = data;
                }
            }
        }

        let sniffed_mime_opt = self.db.get_mime_type_for_data(&self.data);

        if let Some(name_mime_types_opt) = name_mime_types_opt {
            if let Some((mime, priority)) = sniffed_mime_opt {
                // From the content type guessing implementation in GIO:
                //
                // For security reasons we don't ever want to sniff desktop files
                // where we know the filename and it doesn't have a .desktop extension.
                // This is because desktop files allow executing any application and
                // we don't want to make it possible to hide them looking like something
                // else.
                if mime == "application/x-desktop" {
                    return Guess::new_certain(mime::TEXT_PLAIN);
                }

                // We found a match with a high confidence value
                if priority >= 80 || name_mime_types_opt.is_none() {
                    return Guess::new_certain(mime);
                }

                if let Some(name_mime_types) = name_mime_types_opt.as_ref() {
                    // We have possible conflicts, but the data matches the
                    // file name, so let's see if the sniffed MIME type is
                    // a subclass of the MIME type associated to the file name,
                    // and use that as a tie breaker.
                    if name_mime_types
                        .iter()
                        .any(|m| self.db.mime_type_subclass(&mime, m))
                    {
                        return Guess::new_certain(mime);
                    }
                }
            } else if !self.data.is_empty() && looks_like_text(&self.data) {
                // "If no magic rule matches the data (or if the content is not
                // available), use the default type of application/octet-stream
                // for binary data, or text/plain for textual data."
                // -- shared-mime-info, "Recommended checking order"
                return Guess::new_certain(mime::TEXT_PLAIN);
            }

            if let Some(name_mime_types) = name_mime_types_opt {
                // If there are conflicts, and the data does not help us,
                // we just pick the first result
                return Guess {
                    mime: name_mime_types.into_iter().next().unwrap(),
                    uncertain: true,
                };
            }
        } else if !self.data.is_empty() {
            // No name but has data => return sniffed mime
            if let Some((mime, _)) = sniffed_mime_opt {
                return Guess::new_certain(mime);
            }
        }
        // No names and no data => unknown MIME type

        Guess {
            mime: mime::APPLICATION_OCTET_STREAM,
            uncertain: true,
        }
    }
}

fn looks_like_text(data: &[u8]) -> bool {
    // "Checking the first 128 bytes of the file for ASCII
    // control characters is a good way to guess whether a
    // file is binary or text."
    // -- shared-mime-info, "Recommended checking order"
    data.iter()
        .take(128)
        .all(|ch| !ch.is_ascii_control() || ch.is_ascii_whitespace())
}

impl Guess {
    /// The guessed MIME type.
    pub const fn mime_type(&self) -> &mime::Mime {
        &self.mime
    }

    /// Whether the guessed MIME type is uncertain.
    ///
    /// If the MIME type was guessed only from its file name there can be
    /// multiple matches, but the [`mime_type`] method will return just the
    /// first match.
    ///
    /// If you only have a file name, and you want to gather all potential
    /// matches, you should use the [`get_mime_types_from_file_name`] method
    /// instead of performing a guess.
    ///
    /// [`mime_type`]: #method.mime_type
    /// [`get_mime_types_from_file_name`]: struct.SharedMimeInfo.html#method.get_mime_types_from_file_name
    pub const fn uncertain(&self) -> bool {
        self.uncertain
    }

    const fn new_certain(mime: mime::Mime) -> Self {
        Self {
            mime,
            uncertain: false,
        }
    }
}

impl Default for SharedMimeInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMimeInfo {
    fn create() -> SharedMimeInfo {
        SharedMimeInfo {
            aliases: alias::AliasesList::new(),
            parents: parent::ParentsMap::new(),
            icons: Vec::new(),
            generic_icons: Vec::new(),
            globs: glob::GlobMap::new(),
            magic: Vec::new(),
            mime_dirs: Vec::new(),
        }
    }

    fn load_directory(&mut self, mut directory: PathBuf) -> io::Result<()> {
        directory.push("mime");
        let mime_path = directory.into_boxed_path();

        self.aliases.add_aliases_from_dir(&mime_path)?;
        self.parents.add_subclasses_from_dir(&mime_path)?;
        self.globs.add_globs_from_dir(&mime_path)?;
        icon::add_icons_from_dir(&mime_path, false, &mut self.icons)?;
        icon::add_icons_from_dir(&mime_path, true, &mut self.generic_icons)?;

        let mut magic_entries = magic::read_magic_from_dir(&mime_path)?;
        self.magic.append(&mut magic_entries);

        let mtime = fs::metadata(&mime_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());

        self.mime_dirs.push(MimeDirectory {
            path: mime_path,
            mtime,
        });

        Ok(())
    }

    /// Creates a new `SharedMimeInfo` instance containing all MIME information
    /// under the [standard XDG base directories][xdg-basedir].
    ///
    /// [xdg-basedir]: http://standards.freedesktop.org/basedir-spec/basedir-spec-latest.html
    pub fn new() -> SharedMimeInfo {
        let mut db = SharedMimeInfo::create();

        let home_dir = env::home_dir();
        let data_home = env::var_os("XDG_DATA_HOME")
            .and_then(|path| {
                let path = PathBuf::from(path);
                path.is_absolute().then_some(path)
            })
            .or_else(|| {
                home_dir.map(|mut home_dir| {
                    home_dir.push(".local/share");
                    home_dir
                })
            })
            .expect("Expected XDG_DATA_HOME or HOME to be set");
        db.load_directory(data_home);

        let mut loaded_data_dirs = false;
        if let Some(paths) = env::var_os("XDG_DATA_DIRS") {
            for path in env::split_paths(&paths).filter(|p| p.is_absolute()) {
                loaded_data_dirs |= db.load_directory(path).is_ok();
            }
        }

        if !loaded_data_dirs {
            db.load_directory("/usr/local/share".into());
            db.load_directory("/usr/share".into());
        }

        db
    }

    /// Loads all the MIME information under `directory`, and creates a new
    /// [`SharedMimeInfo`] instance for it.
    ///
    /// This method is only really useful for testing purposes; you should
    /// always use the [`new`] method, instead.
    ///
    /// [`SharedMimeInfo`]: struct.SharedMimeInfo.html
    /// [`new`]: #method.new
    pub fn new_for_directory<P: AsRef<Path>>(directory: P) -> SharedMimeInfo {
        let mut db = SharedMimeInfo::create();

        db.load_directory(directory.as_ref().to_path_buf());

        db
    }

    /// Reloads the contents of the [`SharedMimeInfo`] type from the directories
    /// used to populate it at construction time. You should use this method
    /// if you're planning to keep the database around for long running operations
    /// or applications.
    ///
    /// This method does not do anything if the directories haven't changed
    /// since the time they were loaded last.
    ///
    /// This method will return `true` if the contents of the shared MIME
    /// database were updated.
    ///
    /// [`SharedMimeInfo`]: struct.SharedMimeInfo.html
    pub fn reload(&mut self) -> bool {
        let mut dropped_db = false;

        // Do not reload the data if nothing has changed
        for dir in &self.mime_dirs {
            let mtime =
                fs::metadata(&dir.path).map_or(dir.mtime, |v| v.modified().unwrap_or(dir.mtime));

            // Drop everything if a directory was changed since
            // the last time we looked into it
            if dir.mtime < mtime {
                dropped_db = true;
                break;
            }
        }

        if dropped_db {
            for dir in &mut self.mime_dirs {
                self.aliases.clear();
                self.aliases.add_aliases_from_dir(&dir.path);

                self.parents.clear();
                self.parents.add_subclasses_from_dir(&dir.path);

                self.globs.clear();
                self.globs.add_globs_from_dir(&dir.path);

                self.icons.clear();
                icon::add_icons_from_dir(&dir.path, false, &mut self.icons);
                self.generic_icons.clear();
                icon::add_icons_from_dir(&dir.path, true, &mut self.generic_icons);

                if let Ok(magic) = magic::read_magic_from_dir(&dir.path) {
                    self.magic = magic;
                };

                dir.mtime = fs::metadata(&dir.path)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or_else(|_| SystemTime::now());
            }
        }

        dropped_db
    }

    /// Retrieves the MIME type aliased by a MIME type, if any.
    pub fn unalias_mime_type(&self, mime_type: &Mime) -> Option<Mime> {
        self.aliases.unalias_mime_type(mime_type)
    }

    /// Looks up the icons associated to a MIME type.
    ///
    /// The icons can be looked up within the current [icon theme][xdg-icon-theme].
    ///
    /// [xdg-icon-theme]: https://specifications.freedesktop.org/icon-theme-spec/icon-theme-spec-latest.html
    pub fn lookup_icon_names(&self, mime_type: &Mime) -> Vec<String> {
        let mut res = Vec::new();

        if let Some(v) = icon::find_icon(&self.icons, mime_type) {
            res.push(v.to_string());
        }

        res.push(mime_type.essence_str().replace('/', "-"));

        if let Some(v) = icon::find_icon(&self.generic_icons, mime_type) {
            res.push(v.to_string());
        } else {
            let generic = format!("{}-x-generic", mime_type.type_());
            res.push(generic);
        }

        res
    }

    /// Looks up the generic icon associated to a MIME type.
    ///
    /// The icon can be looked up within the current [icon theme][xdg-icon-theme].
    ///
    /// [xdg-icon-theme]: https://specifications.freedesktop.org/icon-theme-spec/icon-theme-spec-latest.html
    pub fn lookup_generic_icon_name(&self, mime_type: &Mime) -> Option<String> {
        let res = icon::find_icon(&self.generic_icons, mime_type).map_or_else(
            || format!("{}-x-generic", mime_type.type_()),
            str::to_string,
        );

        Some(res)
    }

    /// Retrieves all the parent MIME types associated to `mime_type`.
    pub fn get_parents(&self, mime_type: &Mime) -> Option<Vec<Mime>> {
        let unaliased = self.aliases.unalias_mime_type(mime_type)?;

        let mut res = vec![unaliased.clone()];

        if let Some(parents) = self.parents.lookup(&unaliased) {
            res.extend(parents.iter().cloned());
        }

        Some(res)
    }

    /// Retrieves all the parent MIME types associated to `mime_type`, without unaliasing.
    /// You may want to use this to get a list of sub-classes, eg:
    ///
    /// ```rust
    /// # use mime::Mime;
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let mime = "application/toml".parse().unwrap();
    /// let parents = mime_db.get_parents_aliased(&mime).unwrap();
    /// assert!(parents.contains(&mime::TEXT_PLAIN));
    /// ```
    pub fn get_parents_aliased(&self, mime_type: &Mime) -> Option<Vec<Mime>> {
        self.parents
            .lookup(mime_type)
            .filter(|&res| !res.is_empty())
            .map(|res| res.iter().cloned().collect())
    }

    /// Retrieves the list of matching MIME types for the given file name,
    /// without looking at the data inside the file.
    ///
    /// If no specific MIME-type can be determined, returns a single
    /// element vector containing the `application/octet-stream` MIME type.
    ///
    /// ```rust
    /// # use std::error::Error;
    /// # use std::str::FromStr;
    /// # use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let mime_types: Vec<Mime> = mime_db.get_mime_types_from_file_name("file.txt");
    /// assert_eq!(mime_types, vec![Mime::from_str("text/plain")?]);
    /// #
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_mime_types_from_file_name(&self, file_name: &str) -> Vec<Mime> {
        self.globs
            .lookup_mime_type_for_file_name(file_name)
            .unwrap_or_else(|| vec![mime::APPLICATION_OCTET_STREAM])
    }

    /// Retrieves the MIME type for the given data, and the priority of the
    /// match. A priority above 80 means a certain match.
    pub fn get_mime_type_for_data(&self, data: &[u8]) -> Option<(Mime, u32)> {
        if data.is_empty() {
            let empty_mime: mime::Mime = "application/x-zerosize".parse().unwrap();
            return Some((empty_mime, 100));
        }

        magic::lookup_data(&self.magic, data)
    }

    /// Checks whether two MIME types are equal, taking into account
    /// eventual aliases.
    ///
    /// ```rust
    /// # use std::error::Error;
    /// # use std::str::FromStr;
    /// # use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let x_markdown: Mime = "text/x-markdown".parse()?;
    /// let markdown: Mime = "text/markdown".parse()?;
    /// assert!(mime_db.mime_type_equal(&x_markdown, &markdown));
    /// #
    /// # Ok(())
    /// # }
    /// ```
    pub fn mime_type_equal(&self, mime_a: &Mime, mime_b: &Mime) -> bool {
        let unaliased_a = self
            .unalias_mime_type(mime_a)
            .unwrap_or_else(|| mime_a.clone());
        let unaliased_b = self
            .unalias_mime_type(mime_b)
            .unwrap_or_else(|| mime_b.clone());

        unaliased_a == unaliased_b
    }

    /// Checks whether a MIME type is a subclass of another MIME type.
    ///
    /// ```rust
    /// # use std::error::Error;
    /// # use std::str::FromStr;
    /// # use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let rust: Mime = "text/rust".parse()?;
    /// let text: Mime = "text/plain".parse()?;
    /// assert!(mime_db.mime_type_subclass(&rust, &text));
    /// #
    /// # Ok(())
    /// # }
    /// ```
    pub fn mime_type_subclass(&self, mime_type: &Mime, base: &Mime) -> bool {
        let unaliased_mime = self
            .unalias_mime_type(mime_type)
            .unwrap_or_else(|| mime_type.clone());
        let unaliased_base = self.unalias_mime_type(base).unwrap_or_else(|| base.clone());

        if unaliased_mime == unaliased_base {
            return true;
        }

        // Handle super-types
        if unaliased_base.subtype() == mime::STAR {
            let base_type = unaliased_base.type_();
            let unaliased_type = unaliased_mime.type_();

            if base_type == unaliased_type {
                return true;
            }
        }

        // The text/plain and application/octet-stream require some
        // special handling:
        //
        //  - All text/* types are subclasses of text/plain.
        //  - All streamable types (ie, everything except the
        //    inode/* types) are subclasses of application/octet-stream
        //
        // https://specifications.freedesktop.org/shared-mime-info-spec/shared-mime-info-spec-latest.html#subclassing
        if unaliased_base == mime::TEXT_PLAIN && unaliased_mime.type_() == mime::TEXT {
            return true;
        }

        if unaliased_base == mime::APPLICATION_OCTET_STREAM && unaliased_mime.type_() != "inode" {
            return true;
        }

        if let Some(parents) = self.parents.lookup(&unaliased_mime) {
            if parents
                .iter()
                .any(|p| self.mime_type_subclass(p, &unaliased_base))
            {
                return true;
            }
        }

        false
    }

    /// Creates a new [`GuessBuilder`] that can be used to guess the MIME type
    /// of a file name, its contents, or a path.
    ///
    /// ```rust
    /// # use std::error::Error;
    /// # use std::str::FromStr;
    /// # use mime::Mime;
    /// #
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # let mime_db = xdg_mime::SharedMimeInfo::new();
    /// // let mime_db = ...
    /// let mut gb = mime_db.guess_mime_type();
    /// let guess = gb.file_name("foo.txt").guess();
    /// assert_eq!(*guess.mime_type(), mime::TEXT_PLAIN);
    /// assert_eq!(guess.uncertain(), false);
    /// #
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`GuessBuilder`]: struct.GuessBuilder.html
    pub const fn guess_mime_type(&self) -> GuessBuilder<'_> {
        GuessBuilder {
            db: self,
            file_name: None,
            data: Vec::new(),
            metadata: None,
            path: None,
            zero_size: true,
        }
    }
}

/// A helper function that reads the entirety of a file to a string,
/// then parses each line using `f`, extending `collection` from those lines.
///
/// # Errors
/// This function returns an error if and only if [`std::fs::read_to_string`] does.
fn extend_from_path<T: Extend<A>, A>(
    collection: &mut T,
    path: &Path,
    f: fn(&str) -> Option<A>,
) -> io::Result<()> {
    let file = fs::read_to_string(path)?;

    collection.extend(
        file.lines()
            .filter(|&line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(f),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::str::FromStr;

    fn load_test_data() -> SharedMimeInfo {
        SharedMimeInfo::new_for_directory("test_files")
    }

    #[test]
    fn load_from_directory() {
        let mut cwd = env::current_dir().unwrap();
        cwd.push("test_files");
        SharedMimeInfo::new_for_directory(cwd);
    }

    #[test]
    fn load_system() {
        let _db = SharedMimeInfo::new();
    }

    #[test]
    fn load_default() {
        let _db = SharedMimeInfo::default();
    }

    #[test]
    fn reload() {
        // We don't load the system data in the, admittedly, remote case the system
        // is getting updated *while* we run the test suite.
        let mut db = load_test_data();

        assert!(!db.reload());
    }

    #[test]
    fn lookup_generic_icons() {
        let mime_db = load_test_data();

        assert_eq!(
            mime_db.lookup_generic_icon_name(&mime::APPLICATION_JSON),
            Some("text-x-script".to_string())
        );
        assert_eq!(
            mime_db.lookup_generic_icon_name(&mime::TEXT_PLAIN),
            Some("text-x-generic".to_string())
        );
    }

    #[test]
    fn unalias() {
        let mime_db = load_test_data();

        assert_eq!(
            mime_db.unalias_mime_type(&"application/ics".parse().unwrap()),
            Some(Mime::from_str("text/calendar").unwrap())
        );
        assert_eq!(
            mime_db.unalias_mime_type(&"text/plain".parse().unwrap()),
            None
        );
    }

    #[test]
    fn mime_type_equal() {
        let mime_db = load_test_data();

        assert!(mime_db.mime_type_equal(
            &"application/wordperfect".parse().unwrap(),
            &"application/vnd.wordperfect".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_equal(
            &"application/x-gnome-app-info".parse().unwrap(),
            &"application/x-desktop".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_equal(
            &"application/x-wordperfect".parse().unwrap(),
            &"application/vnd.wordperfect".parse().unwrap(),
        ));
        assert!(!mime_db.mime_type_equal(
            &"application/x-wordperfect".parse().unwrap(),
            &"audio/x-midi".parse().unwrap(),
        ));
        assert!(!mime_db.mime_type_equal(
            &"application/octet-stream".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        assert!(
            !mime_db.mime_type_equal(&"text/plain".parse().unwrap(), &"text/*".parse().unwrap(),)
        );
    }

    #[test]
    fn mime_type_for_file_name() {
        let mime_db = load_test_data();

        assert_eq!(
            mime_db.get_mime_types_from_file_name("foo.txt"),
            vec![Mime::from_str("text/plain").unwrap()]
        );

        assert_eq!(
            mime_db.get_mime_types_from_file_name("bar.gif"),
            vec![Mime::from_str("image/gif").unwrap()]
        );

        assert_eq!(
            mime_db.get_mime_types_from_file_name("baz.mod"),
            vec![Mime::from_str("audio/x-mod").unwrap()]
        );
    }

    #[test]
    fn mime_type_for_file_data() {
        let mime_db = load_test_data();

        let svg_data = include_bytes!("../test_files/files/rust-logo.svg");
        assert_eq!(
            mime_db.get_mime_type_for_data(svg_data),
            Some((Mime::from_str("image/svg+xml").unwrap(), 80))
        );

        let png_data = include_bytes!("../test_files/files/rust-logo.png");
        assert_eq!(
            mime_db.get_mime_type_for_data(png_data),
            Some((Mime::from_str("image/png").unwrap(), 50))
        );
    }

    #[test]
    fn mime_type_subclass() {
        let mime_db = load_test_data();

        assert!(mime_db.mime_type_subclass(
            &"application/rtf".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"message/news".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"message/news".parse().unwrap(),
            &"message/*".parse().unwrap(),
        ));
        assert!(mime_db
            .mime_type_subclass(&"message/news".parse().unwrap(), &"text/*".parse().unwrap(),));
        assert!(mime_db.mime_type_subclass(
            &"message/news".parse().unwrap(),
            &"application/octet-stream".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"application/rtf".parse().unwrap(),
            &"application/octet-stream".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"application/x-gnome-app-info".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"image/x-djvu".parse().unwrap(),
            &"image/vnd.djvu".parse().unwrap(),
        ));
        assert!(mime_db.mime_type_subclass(
            &"image/vnd.djvu".parse().unwrap(),
            &"image/x-djvu".parse().unwrap(),
        ));
        assert!(!mime_db.mime_type_subclass(
            &"image/vnd.djvu".parse().unwrap(),
            &"text/plain".parse().unwrap(),
        ));
        assert!(!mime_db.mime_type_subclass(
            &"image/vnd.djvu".parse().unwrap(),
            &"text/*".parse().unwrap(),
        ));
        assert!(
            mime_db.mime_type_subclass(&"text/*".parse().unwrap(), &"text/plain".parse().unwrap())
        );
        assert!(mime_db.mime_type_subclass(
            &"application/x-shellscript".parse().unwrap(),
            &mime::APPLICATION_OCTET_STREAM
        ));
    }

    #[test]
    fn guess_none() {
        let mime_db = load_test_data();

        let mut gb = mime_db.guess_mime_type();
        let guess = gb.guess();
        assert_eq!(*guess.mime_type(), mime::APPLICATION_OCTET_STREAM);
        assert!(guess.uncertain());
    }

    #[test]
    fn guess_filename() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.file_name("foo.txt").guess();
        assert_eq!(*guess.mime_type(), mime::TEXT_PLAIN);
        assert!(!guess.uncertain());
    }

    #[test]
    fn guess_data() {
        let svg_data = include_bytes!("../test_files/files/rust-logo.svg");
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.data(svg_data).guess();
        assert_eq!(*guess.mime_type(), "image/svg+xml");
        assert!(!guess.uncertain());
    }

    #[test]
    fn guess_both() {
        let png_data = include_bytes!("../test_files/files/rust-logo.png");
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.file_name("rust-logo.png").data(png_data).guess();
        assert_eq!(*guess.mime_type(), "image/png");
        assert!(!guess.uncertain());
    }

    #[test]
    fn guess_script() {
        let sh_data = include_bytes!("../test_files/files/script");
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.data(sh_data).guess();
        assert_eq!(*guess.mime_type(), "application/x-shellscript");
    }

    #[test]
    fn guess_script_with_name() {
        let sh_data = include_bytes!("../test_files/files/gp");
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.file_name("gp").data(sh_data).guess();
        assert_eq!(*guess.mime_type(), "application/x-shellscript");
    }

    #[test]
    fn guess_empty() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.path("test_files/files/empty").guess();
        assert_ne!(*guess.mime_type(), mime::TEXT_PLAIN);
        assert_eq!(*guess.mime_type(), "application/x-zerosize");
    }

    #[test]
    fn guess_empty_no_zero_size() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let file = "test_files/files/empty.json";
        let guess = gb.path(file).guess();
        assert_ne!(*guess.mime_type(), mime::APPLICATION_JSON);
        assert_eq!(*guess.mime_type(), "application/x-zerosize");
        let guess = gb.path(file).zero_size(false).guess();
        assert_ne!(*guess.mime_type(), "application/x-zerosize");
        assert_eq!(*guess.mime_type(), mime::APPLICATION_JSON);
    }

    #[test]
    fn guess_empty_no_zero_size_no_extension() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let file = "test_files/files/empty";
        let guess = gb.path(file).guess();
        assert_eq!(*guess.mime_type(), "application/x-zerosize");
        let guess = gb.path(file).zero_size(false).guess();
        // Return `application/x-zerosize` even if `zero_size` is false
        assert_eq!(*guess.mime_type(), "application/x-zerosize");
    }

    #[test]
    fn guess_text() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.path("test_files/files/text").guess();
        assert_eq!(*guess.mime_type(), mime::TEXT_PLAIN);
    }

    #[test]
    fn looks_like_text_works() {
        assert!(looks_like_text(&[]));
        assert!(looks_like_text(b"hello"));
        assert!(!looks_like_text(b"hello\x00"));
        assert!(!looks_like_text(&[0, 1, 2]));
    }

    #[test]
    fn guess_turtle() {
        let ttl_data = include_bytes!("../test_files/files/example.ttl");
        let ttl_meta = std::fs::metadata("test_files/files/example.ttl").unwrap();
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb
            .file_name("example.ttl")
            .metadata(ttl_meta)
            .data(ttl_data)
            .guess();
        assert_eq!(*guess.mime_type(), "text/turtle");
    }

    #[test]
    fn guess_dodgy_desktop_file() {
        let desktop_data = include_bytes!("../test_files/files/launcher");
        let desktop_meta = std::fs::metadata("test_files/files/launcher").unwrap();
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb
            .file_name("launcher")
            .metadata(desktop_meta)
            .data(desktop_data)
            .guess();
        assert_eq!(*guess.mime_type(), "text/plain");
    }

    #[test]
    fn guess_html_with_no_html_tags() {
        let mime_db = load_test_data();
        let mut gb = mime_db.guess_mime_type();
        let guess = gb.path("test_files/files/no_html_tags.html").guess();
        assert_eq!(*guess.mime_type(), mime::TEXT_HTML);
    }
}
