use oxikube_extension_api::{self as oxikube, Extension};

struct {{type_name}};

impl Extension for {{type_name}} {
    fn new() -> Self {
        Self
    }

    fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
        match command_id {
            "say-hello" => {
                // Host calls are capability-gated: add a grant to extension.toml before using
                // `process`, `http_client` or `kube_info`.
                oxikube::key_value_store::insert("last-args", &args.join(" "))
                    .map_err(|err| format!("{err:?}"))
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

oxikube::register_extension!({{type_name}});
