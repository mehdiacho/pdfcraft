//! The part of rfd's `AsyncFileDialog` API that PdfCraft uses, for Android, where rfd has no
//! backend.
//!
//! `pdfcraft-ui-egui` depends on this crate under the name `rfd` when it targets Android, so
//! every picker in the app keeps its code. Unlike rfd, the dialog's title, filters and
//! suggested file name are readable: the Android app turns them into a Storage Access
//! Framework request (MIME types, the document title) through the [`Backend`] it installs.
//!
//! The chosen files arrive as paths. Android hands out `content://` URIs, so the app copies an
//! opened file into its cache and gives that path; for Save and folder picks it gives a staging
//! path and copies what is written there to the chosen document or folder.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

/// Which system picker to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One existing file.
    File,
    /// One or more existing files.
    Files,
    /// A folder to write into.
    Folder,
    /// Where to save a new file.
    Save,
}

/// One filter: a label and its extensions, without dots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// What the app asked for.
#[derive(Clone, Debug)]
pub struct Request {
    pub kind: Kind,
    pub title: Option<String>,
    pub filters: Vec<Filter>,
    pub file_name: Option<String>,
}

impl Request {
    /// The MIME types for the request's filters, for the system picker. Any extension without
    /// a known type widens the picker to every file, as does a request without filters.
    pub fn mime_types(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for ext in self.filters.iter().flat_map(|f| &f.extensions) {
            let Some(mime) = mime_for(ext) else {
                return vec!["*/*".to_string()];
            };
            if !out.iter().any(|m| m == mime) {
                out.push(mime.to_string());
            }
        }
        if out.is_empty() {
            out.push("*/*".to_string());
        }
        out
    }
}

fn mime_for(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "txt" | "text" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "rtf" => "application/rtf",
        "json" => "application/json",
        "xml" | "xfdf" => "application/xml",
        "fdf" => "application/vnd.fdf",
        "p12" | "pfx" => "application/x-pkcs12",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return None,
    })
}

/// Shows a picker for `Request` and sends the chosen paths (empty when cancelled) on the sender.
/// Called on the thread that asked; it must not block.
pub type Backend = Arc<dyn Fn(Request, mpsc::Sender<Vec<PathBuf>>) + Send + Sync>;

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// Install the picker backend, once, before the app asks for a file.
pub fn set_backend(backend: Backend) -> bool {
    BACKEND.set(backend).is_ok()
}

/// A chosen file or folder.
#[derive(Clone, Debug)]
pub struct FileHandle(PathBuf);

impl FileHandle {
    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn file_name(&self) -> String {
        self.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// A picker request under construction (rfd's builder methods).
#[derive(Clone, Debug, Default)]
pub struct AsyncFileDialog {
    title: Option<String>,
    filters: Vec<Filter>,
    file_name: Option<String>,
}

impl AsyncFileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push(Filter { name: name.into(), extensions: extensions.iter().map(ToString::to_string).collect() });
        self
    }

    #[must_use]
    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub fn pick_file(self) -> impl Future<Output = Option<FileHandle>> {
        let rx = self.show(Kind::File);
        async move { first(wait(rx)) }
    }

    pub fn pick_files(self) -> impl Future<Output = Option<Vec<FileHandle>>> {
        let rx = self.show(Kind::Files);
        async move {
            let paths = wait(rx);
            (!paths.is_empty()).then(|| paths.into_iter().map(FileHandle).collect())
        }
    }

    pub fn pick_folder(self) -> impl Future<Output = Option<FileHandle>> {
        let rx = self.show(Kind::Folder);
        async move { first(wait(rx)) }
    }

    pub fn save_file(self) -> impl Future<Output = Option<FileHandle>> {
        let rx = self.show(Kind::Save);
        async move { first(wait(rx)) }
    }

    /// Ask now, on the calling (UI) thread, like rfd does on macOS.
    fn show(self, kind: Kind) -> Option<mpsc::Receiver<Vec<PathBuf>>> {
        let backend = BACKEND.get()?;
        let (tx, rx) = mpsc::channel();
        backend(Request { kind, title: self.title, filters: self.filters, file_name: self.file_name }, tx);
        Some(rx)
    }
}

/// Block until the backend answers. The app awaits these futures on its file-picker worker
/// thread, never on the UI thread, so blocking here is what rfd's own backends do too.
fn wait(rx: Option<mpsc::Receiver<Vec<PathBuf>>>) -> Vec<PathBuf> {
    rx.and_then(|rx| rx.recv().ok()).unwrap_or_default()
}

fn first(paths: Vec<PathBuf>) -> Option<FileHandle> {
    paths.into_iter().next().map(FileHandle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(exts: &[&[&str]]) -> Request {
        Request {
            kind: Kind::File,
            title: None,
            filters: exts.iter().map(|e| Filter { name: String::new(), extensions: e.iter().map(|s| s.to_string()).collect() }).collect(),
            file_name: None,
        }
    }

    #[test]
    fn known_extensions_become_mime_types() {
        assert_eq!(request(&[&["pdf"], &["png", "JPG", "jpeg"]]).mime_types(), ["application/pdf", "image/png", "image/jpeg"]);
    }

    #[test]
    fn an_unknown_extension_or_no_filter_allows_every_file() {
        assert_eq!(request(&[&["pdf"], &["weird"]]).mime_types(), ["*/*"]);
        assert_eq!(request(&[]).mime_types(), ["*/*"]);
    }

    #[test]
    fn no_backend_reads_as_cancelled() {
        let picked = std::thread::spawn(|| futures_block_on(AsyncFileDialog::new().add_filter("PDF", &["pdf"]).pick_file()));
        assert!(picked.join().unwrap().is_none());
    }

    /// A minimal executor: the futures here never return Pending.
    fn futures_block_on<F: Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => unreachable!("picker futures complete on first poll"),
        }
    }
}
