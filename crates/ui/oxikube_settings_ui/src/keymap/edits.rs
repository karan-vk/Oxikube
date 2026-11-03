//! `keymap::SetBinding` and `keymap::RemoveBinding` (E21-S08): the bus handlers turn the payload
//! into `oxikube_keymap::edit` operations and hand them to the UI thread, which writes them
//! ([`oxikube_keymap::edit::write_user_keymap`]: off the UI thread, validated, in force at once)
//! and answers with the result, so the palette, the recorder and an agent's tool call all see a
//! refused edit as the command's error.
//!
//! Neither is a mutation: they change the user's `keymap.json` only.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use gpui::{App, Task};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_keymap::edit::{
    KeybindUpdateOperation, remove_binding_operations, set_binding_operations, write_user_keymap,
};

/// An edit of `keymap.json` waiting for the UI thread, and where its result goes.
pub struct KeymapEditRequest {
    /// The operations to write.
    pub operations: Vec<KeybindUpdateOperation>,
    /// Receives the write's result.
    pub reply: oneshot::Sender<OxiResult<()>>,
}

/// A handle on the app's keymap-edit queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct KeymapEditSink {
    tx: UnboundedSender<KeymapEditRequest>,
}

impl KeymapEditSink {
    /// A sink and the receiver [`KeymapEdits::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<KeymapEditRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Write `operations` and wait for the result.
    ///
    /// # Errors
    ///
    /// The write's error, or an internal error when the app is gone.
    pub async fn apply(&self, operations: Vec<KeybindUpdateOperation>) -> OxiResult<()> {
        let (reply, result) = oneshot::channel();
        self.tx
            .unbounded_send(KeymapEditRequest { operations, reply })
            .map_err(|_| OxiError::internal("the keymap editor is gone"))?;
        result
            .await
            .map_err(|_| OxiError::internal("the keymap edit was dropped"))?
    }
}

/// Registers `keymap::SetBinding` and `keymap::RemoveBinding` (tool stubs
/// `app.keymap_set_binding`, `app.keymap_remove_binding`) on `registry`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is already taken.
pub fn register_edit_commands(
    registry: &mut CommandRegistry,
    sink: KeymapEditSink,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::KEYMAP_SET_BINDING,
        CommandId::KEYMAP_REMOVE_BINDING,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let operations = match &command {
                    Command::KeymapSetBinding { edit } => set_binding_operations(edit)
                        .map_err(|err| OxiError::validation(err.to_string()))?,
                    Command::KeymapRemoveBinding { binding } => remove_binding_operations(binding),
                    _ => return Err(OxiError::validation(format!("not a {id} command"))),
                };
                sink.apply(operations).await?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}

/// The app's keymap edits.
pub struct KeymapEdits;

impl KeymapEdits {
    /// Starts writing the requests of `requests`, one at a time, in order. Keep the returned task
    /// for the life of the app (dropping it stops it).
    pub fn start(mut requests: UnboundedReceiver<KeymapEditRequest>, cx: &mut App) -> Task<()> {
        cx.spawn(async move |cx| {
            while let Some(request) = requests.next().await {
                let write = cx.update(|cx| write_user_keymap(cx, request.operations));
                let result = write.await;
                if let Err(err) = &result {
                    tracing::warn!(%err, "keymap edit refused");
                }
                // The caller may have stopped waiting; the edit stands either way.
                request.reply.send(result).ok();
            }
        })
    }
}
