use oxikube_extension_api::{self as oxikube, Command, Extension};

struct {{type_name}};

impl Extension for {{type_name}} {
    fn new() -> Self {
        Self
    }

    fn context_server_command(&mut self, context_server_id: &str) -> Result<Command, String> {
        match context_server_id {
            "{{id}}" => Ok(Command {
                // Replace with the MCP server that speaks MCP over stdio.
                command: "my-mcp-server".to_string(),
                args: vec!["--stdio".to_string()],
                env: Vec::new(),
            }),
            other => Err(format!("unknown context server `{other}`")),
        }
    }
}

oxikube::register_extension!({{type_name}});
