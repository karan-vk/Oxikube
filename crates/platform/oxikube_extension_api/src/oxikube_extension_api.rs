//! `oxikube_extension_api` — layer: `platform`.
//!
//! The SDK an Oxikube extension is written against. An extension is a WASM component built for
//! `wasm32-wasip2` that implements the WIT world `oxikube:extension/extension@0.1.0`, whose
//! files live in this crate under `wit/since_v0.1.0/`. It contributes commands and MCP context
//! servers (plus themes declared in `extension.toml`); it never contributes UI (ADR 0007).
//!
//! An extension implements [`Extension`] and registers it with [`register_extension!`]. Host
//! services are the modules [`settings`], [`key_value_store`], [`http_client`], [`process`],
//! [`platform`] and [`kube_info`]; calls the extension has no grant for fail with
//! [`Error::Denied`].
//!
//! ```no_run
//! use oxikube_extension_api::{self as oxikube, Command, Extension};
//!
//! struct Hello;
//!
//! impl Extension for Hello {
//!     fn new() -> Self {
//!         Hello
//!     }
//!
//!     fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
//!         match command_id {
//!             "hello" => Ok(()),
//!             other => Err(format!("unknown command {other} {args:?}")),
//!         }
//!     }
//!
//!     fn context_server_command(&mut self, _id: &str) -> Result<Command, String> {
//!         Ok(Command { command: "my-mcp-server".into(), args: vec!["--stdio".into()], env: vec![] })
//!     }
//! }
//!
//! oxikube::register_extension!(Hello);
//! ```
//!
//! The extension crate sets `crate-type = ["cdylib"]` and builds with
//! `cargo build --target wasm32-wasip2`, which links a component directly.

use std::cell::RefCell;

/// Bindings generated from `wit/since_v0.1.0`. `init-extension` is skipped here because
/// [`register_extension!`] exports it from the extension's own crate.
mod wit {
    wit_bindgen::generate!({
        path: "wit/since_v0.1.0",
        world: "extension",
        skip: ["init-extension"],
    });

    use super::Component;
    export!(Component);
}

pub use wit::oxikube::extension::common::{Command, Error};
pub use wit::oxikube::extension::{
    http_client, key_value_store, kube_info, platform, process, settings,
};

/// An Oxikube extension. One value lives for the lifetime of the loaded component.
pub trait Extension: 'static {
    /// Creates the extension. Called once, from `init-extension`, before any other method.
    fn new() -> Self
    where
        Self: Sized;

    /// Runs the command `command_id`, one of the ids the manifest's `commands` table declares.
    /// An `Err` is shown to the user.
    fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
        let _ = args;
        Err(format!("unknown command `{command_id}`"))
    }

    /// Returns how to start the MCP context server `context_server_id`, one of the ids the
    /// manifest's `context_servers` table declares. The host spawns it and speaks MCP over stdio.
    fn context_server_command(&mut self, context_server_id: &str) -> Result<Command, String> {
        Err(format!("unknown context server `{context_server_id}`"))
    }
}

/// Registers `$extension_type` as this component's [`Extension`] by exporting
/// `init-extension`. Invoke it exactly once, at the root of the extension crate.
#[macro_export]
macro_rules! register_extension {
    ($extension_type:ty) => {
        /// The component's `init-extension` export.
        #[unsafe(export_name = "init-extension")]
        pub extern "C" fn __oxikube_init_extension() {
            $crate::register_extension(|| Box::new(<$extension_type as $crate::Extension>::new()));
        }
    };
}

thread_local! {
    /// The registered extension. WASM components are single-threaded.
    static EXTENSION: RefCell<Option<Box<dyn Extension>>> = RefCell::new(None);
}

/// Stores the extension built by `build`. Called by [`register_extension!`]; not for direct use.
#[doc(hidden)]
pub fn register_extension(build: fn() -> Box<dyn Extension>) {
    EXTENSION.with(|slot| *slot.borrow_mut() = Some(build()));
}

/// Runs `f` on the registered extension, or fails when `init-extension` has not run.
fn with_extension<T>(f: impl FnOnce(&mut dyn Extension) -> Result<T, String>) -> Result<T, String> {
    EXTENSION.with(|slot| match slot.borrow_mut().as_mut() {
        Some(extension) => f(extension.as_mut()),
        None => Err("extension not initialised: register_extension! was not called".to_string()),
    })
}

/// Implements the world's exports by forwarding to the registered [`Extension`].
struct Component;

impl wit::Guest for Component {
    fn run_command(command_id: String, args: Vec<String>) -> Result<(), String> {
        with_extension(|extension| extension.run_command(&command_id, args))
    }

    fn context_server_command(context_server_id: String) -> Result<Command, String> {
        with_extension(|extension| extension.context_server_command(&context_server_id))
    }
}

#[cfg(test)]
mod tests;
