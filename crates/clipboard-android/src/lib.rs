//! The part of arboard's API that PdfCraft uses, for Android, where arboard has no backend.
//!
//! `pdfcraft-ui-egui` uses this crate under the name `arboard` when it targets Android. Every
//! call reports that the clipboard isn't available, which the app already handles (copying
//! an image or a file shows the error; Create ▸ Clipboard finds nothing). Text copy and paste
//! still work: egui hands them to the platform itself.

use std::borrow::Cow;
use std::path::Path;

/// Why a clipboard call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This build has no clipboard for images and files.
    ClipboardNotSupported,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("copying images and files isn't available on Android yet")
    }
}

impl std::error::Error for Error {}

/// RGBA pixels, row by row.
#[derive(Clone, Debug)]
pub struct ImageData<'a> {
    pub width: usize,
    pub height: usize,
    pub bytes: Cow<'a, [u8]>,
}

/// The system clipboard (never available here).
#[derive(Debug)]
pub struct Clipboard(());

impl Clipboard {
    pub fn new() -> Result<Self, Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn set_image(&mut self, _image: ImageData<'_>) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn get_text(&mut self) -> Result<String, Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn set(&mut self) -> Set<'_> {
        Set(self)
    }
}

/// arboard's `Clipboard::set()` builder.
#[derive(Debug)]
pub struct Set<'a>(#[allow(dead_code)] &'a mut Clipboard);

impl Set<'_> {
    pub fn file_list(self, _paths: &[impl AsRef<Path>]) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }
}
