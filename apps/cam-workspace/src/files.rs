//! Files from the computer, for the import tools.
//!
//! A native build reads a path typed into the import dialog, but a browser has no file system
//! path to type: the page must ask the browser for its own dialog. That is a hidden
//! `<input type=file>` element, whose `change` event fires once the user has chosen a file; a
//! `FileReader` then reads it as an `ArrayBuffer`, and its bytes become a [`PickedFile`] the
//! import dialog applies on the next frame.
//!
//! The answer is kept in a thread-local rather than in the app so that the JavaScript callback,
//! which outlives the frame that started it, only has to hand the bytes over. The app drains the
//! slot every frame ([`take`]) and imports what it finds.

use std::cell::RefCell;

use crate::ribbon::ImportKind;

/// A file the user picked on the computer: the name to report and the bytes to import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickedFile {
    /// The file's name as the browser reports it, shown in the status line.
    pub name: String,
    /// The file's contents, read as they are on disk.
    pub bytes: Vec<u8>,
}

/// What a file dialog answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    /// The user chose a file.
    File(PickedFile),
    /// The dialog was closed without choosing one: nothing to report.
    Cancelled,
    /// The browser could not read the file, or could not show the dialog at all.
    Failed(String),
}

thread_local! {
    /// The answer of the one file dialog that can be open, until the app takes it.
    static ANSWER: RefCell<Option<Picked>> = const { RefCell::new(None) };
}

/// Puts `picked` where the app's next frame will find it, and asks for that frame.
pub fn answer(picked: Picked, ctx: &egui::Context) {
    ANSWER.with(|slot| *slot.borrow_mut() = Some(picked));
    ctx.request_repaint();
}

/// Takes the answer of the last file dialog, if one has arrived since the last call.
pub fn take() -> Option<Picked> {
    ANSWER.with(|slot| slot.borrow_mut().take())
}

/// Forgets any answer left over from a dialog the app no longer shows.
pub fn discard() {
    ANSWER.with(|slot| *slot.borrow_mut() = None);
}

/// The file filter the import dialog offers for `kind`.
pub fn accept(kind: ImportKind) -> String {
    format!(".{}", kind.extension())
}

/// Opens the browser's file dialog for `kind`; the answer arrives in [`take`].
///
/// Only a browser has one: a native window uses the import dialog's path field, so this does
/// nothing there.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick(_kind: ImportKind, _ctx: &egui::Context) {}

/// Opens the browser's file dialog for `kind`; the answer arrives in [`take`].
///
/// The click that calls this is the user's gesture, which is what lets a browser open its file
/// dialog at all. The element is removed as soon as it answers, so it never shows on the page.
#[cfg(target_arch = "wasm32")]
pub fn pick(kind: ImportKind, ctx: &egui::Context) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        answer(Picked::Failed("this page cannot show a file dialog".to_string()), ctx);
        return;
    };
    let Ok(input) = document.create_element("input").map(|element| element.unchecked_into::<web_sys::HtmlInputElement>()) else {
        answer(Picked::Failed("this page cannot show a file dialog".to_string()), ctx);
        return;
    };
    input.set_type("file");
    input.set_accept(&accept(kind));
    // Attached but hidden while the dialog is open: some browsers ignore a click on an element
    // that is not in the page.
    let _ = input.set_attribute("style", "display:none");
    if let Some(body) = document.body() {
        let _ = body.append_child(&input);
    }

    let (element, change_ctx) = (input.clone(), ctx.clone());
    let on_change = Closure::once_into_js(move || {
        element.remove();
        let Some(file) = element.files().and_then(|list| list.item(0)) else {
            answer(Picked::Cancelled, &change_ctx);
            return;
        };
        read(file, change_ctx);
    });
    input.set_onchange(Some(on_change.unchecked_ref()));

    // Closing the dialog fires `cancel`, not `change`, in the browsers that have it.
    let (element, cancel_ctx) = (input.clone(), ctx.clone());
    let on_cancel = Closure::once_into_js(move || {
        element.remove();
        answer(Picked::Cancelled, &cancel_ctx);
    });
    input.set_oncancel(Some(on_cancel.unchecked_ref()));

    input.click();
}

/// Reads `file` as an `ArrayBuffer` and hands its bytes to the app. The `FileReader` answers
/// either `loadend` or `error`; both close the dialog's wait.
#[cfg(target_arch = "wasm32")]
fn read(file: web_sys::File, ctx: egui::Context) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    let name = file.name();
    let Ok(reader) = web_sys::FileReader::new() else {
        answer(Picked::Failed(format!("{name}: this page cannot read files")), &ctx);
        return;
    };

    let (loaded, loaded_ctx, loaded_name) = (reader.clone(), ctx.clone(), name.clone());
    let on_load = Closure::once_into_js(move || {
        let bytes = loaded
            .result()
            .ok()
            .and_then(|buffer| buffer.dyn_into::<js_sys::ArrayBuffer>().ok())
            .map(|buffer| js_sys::Uint8Array::new(&buffer).to_vec());
        match bytes {
            Some(bytes) => answer(Picked::File(PickedFile { name: loaded_name, bytes }), &loaded_ctx),
            None => answer(Picked::Failed(format!("{loaded_name}: the browser read nothing")), &loaded_ctx),
        }
    });
    reader.set_onloadend(Some(on_load.unchecked_ref()));

    let (failed, failed_ctx, failed_name) = (reader.clone(), ctx.clone(), name.clone());
    let on_error = Closure::once_into_js(move || {
        let _ = failed;
        answer(Picked::Failed(format!("{failed_name}: the browser could not read the file")), &failed_ctx);
    });
    reader.set_onerror(Some(on_error.unchecked_ref()));

    // A reader that refuses the file answers neither event: report that here.
    if reader.read_as_array_buffer(&file).is_err() {
        answer(Picked::Failed(format!("{name}: the browser could not read the file")), &ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_taken_once() {
        let ctx = egui::Context::default();
        let file = Picked::File(PickedFile { name: "part.dxf".into(), bytes: vec![0, 1, 2] });
        answer(file.clone(), &ctx);
        assert_eq!(take(), Some(file));
        assert_eq!(take(), None);
    }

    #[test]
    fn discarding_forgets_a_stale_answer() {
        let ctx = egui::Context::default();
        answer(Picked::Cancelled, &ctx);
        discard();
        assert_eq!(take(), None);
    }

    #[test]
    fn the_browser_filter_names_the_imports_extension() {
        assert_eq!(accept(ImportKind::Step), ".step");
        assert_eq!(accept(ImportKind::Dxf), ".dxf");
    }
}
