//! A trivial extension: one `hello` command and one MCP context server. Build it as a component
//! with `cargo build -p oxikube_extension_api --target wasm32-wasip2 --example hello`.

use oxikube_extension_api::{
    Command, Error, Extension, key_value_store, kube_info, platform, register_extension,
};

struct Hello;

impl Extension for Hello {
    fn new() -> Self {
        Hello
    }

    fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
        match command_id {
            "hello" => {
                let (os, arch) = platform::current_platform();
                let context = match kube_info::current_context() {
                    Ok(context) => context.unwrap_or_default(),
                    Err(Error::Denied(capability)) => format!("(no {capability} grant)"),
                    Err(Error::Failed(reason)) => return Err(reason),
                };
                let greeting = format!("hello {args:?} from {os:?}/{arch:?} on {context}");
                key_value_store::insert("last-greeting", &greeting)
                    .map_err(|err| format!("{err:?}"))
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }

    fn context_server_command(&mut self, context_server_id: &str) -> Result<Command, String> {
        match context_server_id {
            "hello-mcp" => Ok(Command {
                command: "hello-mcp".to_string(),
                args: vec!["--stdio".to_string()],
                env: Vec::new(),
            }),
            other => Err(format!("unknown context server `{other}`")),
        }
    }
}

register_extension!(Hello);
