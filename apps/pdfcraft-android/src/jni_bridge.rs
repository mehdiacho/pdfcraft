//! The Kotlin ⇄ Rust bridge.
//!
//! Kotlin → Rust:
//! - `MainActivity.nativeOpenBytes(name, bytes)` queues a file for the app's inbox ("Open with",
//!   share). The queue lives here, in a static, because the Activity can deliver a file before
//!   eframe has created the app.
//! - `MainActivity.nativePickResult(id, paths)` answers a picker request.
//!
//! Rust → Kotlin: `MainActivity.requestPick(id, kind, title, mimeTypes, fileName)` shows a
//! system picker for one of the app's `rfd` requests (`pdfcraft-picker-android`).
//!
//! The only `unsafe` in the app is here: borrowing the JavaVM and Activity pointers that
//! android-activity hands over, and the `no_mangle` symbols JNI looks up by name.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::jlong;
use jni::{JNIEnv, JavaVM};
use pdfcraft_picker_android::{Kind, Request};
use pdfcraft_ui_egui::PdfCraftApp;
use winit::platform::android::activity::AndroidApp;

/// Files waiting for the app (name, bytes), and the egui context to wake once it exists.
#[derive(Default)]
struct Shared {
    inbox: Option<pdfcraft_ui_egui::Inbox>,
    early: Vec<(String, Vec<u8>)>,
    ctx: Option<egui::Context>,
    /// Picker requests waiting for `nativePickResult`, by id.
    picks: HashMap<i64, Sender<Vec<PathBuf>>>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, Shared> {
    shared().lock().unwrap_or_else(PoisonError::into_inner)
}

/// The Java VM and the Activity, for calls into Kotlin.
struct Java {
    vm: JavaVM,
    activity: GlobalRef,
}

static JAVA: OnceLock<Java> = OnceLock::new();

/// Called before eframe starts: keep the VM and Activity, and install the picker backend.
pub(crate) fn init(app: &AndroidApp) {
    // SAFETY: android-activity guarantees both pointers are valid for the life of the process
    // (the VM) and of the `AndroidApp` (the Activity); the global reference taken below keeps
    // the Activity object alive independently of that borrow.
    let java = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.and_then(|vm| {
        let activity = {
            let env = vm.attach_current_thread()?;
            // SAFETY: see above; `activity_as_ptr` is a live `jobject` for the Activity.
            let local = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(local)?
        };
        Ok(Java { vm, activity })
    });
    match java {
        Ok(java) => {
            let _ = JAVA.set(java);
            pdfcraft_picker_android::set_backend(Arc::new(request_pick));
        }
        Err(e) => log::error!("no Java bridge, so file pickers won't open: {e}"),
    }
}

/// Connect the running app: deliver anything that arrived early and route later files to its inbox.
pub(crate) fn attach(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let inbox = app.inbox.clone();
    let mut s = lock();
    let early = std::mem::take(&mut s.early);
    if !early.is_empty() {
        inbox.lock().unwrap_or_else(PoisonError::into_inner).extend(early);
        ctx.request_repaint();
    }
    s.inbox = Some(inbox);
    s.ctx = Some(ctx.clone());
}

fn deliver(name: String, bytes: Vec<u8>) {
    let mut s = lock();
    match &s.inbox {
        Some(inbox) => inbox.lock().unwrap_or_else(PoisonError::into_inner).push((name, bytes)),
        None => s.early.push((name, bytes)),
    }
    if let Some(ctx) = &s.ctx {
        ctx.request_repaint();
    }
}

/// The picker backend: ask Kotlin to show the system picker. A failure answers "cancelled".
fn request_pick(request: Request, answer: Sender<Vec<PathBuf>>) {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    lock().picks.insert(id, answer);
    if let Err(e) = call_request_pick(id, &request) {
        log::error!("couldn't show the file picker: {e}");
        // Dropping the sender reads as a cancelled pick.
        lock().picks.remove(&id);
    }
}

fn call_request_pick(id: i64, request: &Request) -> jni::errors::Result<()> {
    let Some(java) = JAVA.get() else {
        return Err(jni::errors::Error::NullPtr("no Java bridge"));
    };
    let mut env = java.vm.attach_current_thread()?;
    let kind = match request.kind {
        Kind::File => 0,
        Kind::Files => 1,
        Kind::Folder => 2,
        Kind::Save => 3,
    };
    let title = env.new_string(request.title.as_deref().unwrap_or(""))?;
    let file_name = env.new_string(request.file_name.as_deref().unwrap_or(""))?;
    let mimes = request.mime_types();
    let string_class = env.find_class("java/lang/String")?;
    let len = i32::try_from(mimes.len()).unwrap_or(i32::MAX);
    let array = env.new_object_array(len, &string_class, JObject::null())?;
    for (i, mime) in (0..len).zip(&mimes) {
        let s = env.new_string(mime)?;
        env.set_object_array_element(&array, i, s)?;
    }
    env.call_method(
        java.activity.as_obj(),
        "requestPick",
        "(JILjava/lang/String;[Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Long(id), JValue::Int(kind), JValue::Object(&title), JValue::Object(&array), JValue::Object(&file_name)],
    )?;
    Ok(())
}

/// `MainActivity.nativeOpenBytes(String, ByteArray)`.
#[unsafe(no_mangle)]
extern "system" fn Java_com_mehdiacho_pdfcraft_MainActivity_nativeOpenBytes<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    name: JString<'l>,
    bytes: JByteArray<'l>,
) {
    let name: String = env.get_string(&name).map(Into::into).unwrap_or_else(|_| "document.pdf".to_string());
    match env.convert_byte_array(&bytes) {
        Ok(bytes) => deliver(name, bytes),
        Err(e) => log::error!("couldn't read {name} from Java: {e}"),
    }
}

/// `MainActivity.nativePickResult(long, String[])`: the chosen paths, empty when cancelled.
#[unsafe(no_mangle)]
extern "system" fn Java_com_mehdiacho_pdfcraft_MainActivity_nativePickResult<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    id: jlong,
    paths: JObjectArray<'l>,
) {
    let Some(answer) = lock().picks.remove(&id) else {
        return;
    };
    let mut out = Vec::new();
    let len = env.get_array_length(&paths).unwrap_or(0);
    for i in 0..len {
        let Ok(obj) = env.get_object_array_element(&paths, i) else { continue };
        if let Ok(s) = env.get_string(&JString::from(obj)) {
            out.push(PathBuf::from(String::from(s)));
        }
    }
    let _ = answer.send(out);
    if let Some(ctx) = &lock().ctx {
        ctx.request_repaint();
    }
}
