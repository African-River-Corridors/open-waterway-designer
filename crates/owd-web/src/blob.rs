//! A browser `File` as a [`ByteSource`] (COG-1): the worker keeps the `File` and reads the bytes a
//! block needs when the engine asks for them, so a sectioned DEM is never loaded whole.
//!
//! The engine runs inside a Web Worker, where `FileReaderSync` reads a `Blob` synchronously. The
//! page's worker gives a JS function `(start, length) -> Uint8Array` that does
//! `new FileReaderSync().readAsArrayBuffer(file.slice(start, start + length))`; the main thread is
//! never blocked. The same shape as the reach steps' fetch ([`crate::JsFetch`]): no browser types
//! in the Rust code.

use owd_format::sectioned::ByteSource;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// A file read by range through the worker's reader.
pub struct JsBlobSource {
    read: js_sys::Function,
    len: u64,
}

impl JsBlobSource {
    pub fn new(read: js_sys::Function, len: f64) -> JsBlobSource {
        JsBlobSource {
            read,
            len: len.max(0.0) as u64,
        }
    }
}

// SAFETY: the engine runs on one thread, inside one Web Worker (wasm32 has no threads here); the
// `ByteSource` bound asks for `Send + Sync` because native surfaces share a terrain between threads.
// A `JsBlobSource` is made and used only on that one thread.
unsafe impl Send for JsBlobSource {}
unsafe impl Sync for JsBlobSource {}

impl ByteSource for JsBlobSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        let got = self
            .read
            .call2(
                &JsValue::NULL,
                &JsValue::from_f64(offset as f64),
                &JsValue::from_f64(buf.len() as f64),
            )
            .map_err(|e| {
                e.as_string()
                    .or_else(|| e.dyn_ref::<js_sys::Error>().map(|err| err.message().into()))
                    .unwrap_or_else(|| "the file could not be read".into())
            })?;
        let arr = js_sys::Uint8Array::new(&got);
        if arr.length() as usize != buf.len() {
            return Err(format!(
                "the file gave {} bytes at byte {offset}, expected {}: it may have changed or \
                 the page may have lost permission to read it",
                arr.length(),
                buf.len()
            ));
        }
        arr.copy_to(buf);
        Ok(())
    }
}
