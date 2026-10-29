//! The file service over the fakes: a real session manager, the scripted `FakeExecStreamPort`
//! and the in-memory `FakeFsPort`.

mod guard;
mod list;
mod transfer;

use std::sync::Arc;

use oxikube_testkit::{ExecScript, ExecStreamCall, FakeExecStreamPort, FakeFsPort};

use super::{FileService, register_commands};
use crate::command_bus::CommandBus;
use crate::testing::{Harness, id};

pub(super) struct Fixture {
    pub(super) h: Harness,
    pub(super) service: Arc<FileService>,
    pub(super) exec: Arc<FakeExecStreamPort>,
    pub(super) fs: Arc<FakeFsPort>,
}

impl Fixture {
    /// A connected cluster `a` (read-only when asked), with the file commands registered.
    pub(super) fn new(read_only: bool) -> Self {
        let fs = Arc::new(FakeFsPort::new());
        let slot = Arc::new(parking_lot::Mutex::new(None::<Arc<FileService>>));
        let (slot_in, fs_in) = (slot.clone(), fs.clone());
        let h = Harness::with_extra(move |reg, _, manager| {
            let service = Arc::new(FileService::new(manager.clone(), fs_in.clone()));
            *slot_in.lock() = Some(service.clone());
            register_commands(reg, service)
        });
        h.connect("a", read_only);
        let exec = h.connector.ports_for(&id("a")).exec_stream;
        let service = slot.lock().clone().expect("registered");
        Self {
            h,
            service,
            exec,
            fs,
        }
    }

    pub(super) fn bus(&self) -> CommandBus {
        self.h.bus.clone()
    }

    /// The argv of every exec so far.
    pub(super) fn commands(&self) -> Vec<Vec<String>> {
        self.exec
            .recorded_calls()
            .into_iter()
            .filter_map(|call| match call {
                ExecStreamCall::Exec { command, .. } => Some(command),
                ExecStreamCall::Attach { .. } => None,
            })
            .collect()
    }

    pub(super) fn script(&self, script: ExecScript) {
        self.exec.script().exec.push_ok(script);
    }
}

/// A tar archive built with the tar crate: `(path, contents)` regular files.
pub(super) fn archive_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, path, *contents)
            .expect("append");
    }
    builder.into_inner().expect("finish")
}
