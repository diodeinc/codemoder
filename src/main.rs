use anyhow::Result;
use clap::Parser;
use codemoder::{CodeModeConfig, CodeModeProxy};
use rmcp::{ServiceExt, transport::TokioChildProcess};
use tokio::process::Command;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "codemoder")]
#[command(about = "MCP proxy that adds code-mode capability to any MCP server")]
struct Args {
    /// Mode: "replace" to only expose execute_tools, "add" to expose both
    #[arg(long, default_value = "add")]
    mode: String,

    /// Name of the code execution tool
    #[arg(long, default_value = "execute_tools")]
    tool_name: String,

    /// Only include these tools (comma-separated). If not specified, includes all.
    #[arg(long)]
    include_tools: Option<String>,

    /// Command to run the downstream MCP server
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    if args.command.is_empty() {
        anyhow::bail!("Must provide a command to run the downstream MCP server");
    }

    let config = {
        let mut cfg = CodeModeConfig::new().with_tool_name(&args.tool_name);

        cfg = match args.mode.as_str() {
            "replace" => cfg.replace_tools(),
            _ => cfg.add(),
        };

        if let Some(tools) = args.include_tools {
            let tool_list: Vec<String> = tools.split(',').map(|s| s.trim().to_string()).collect();
            cfg = cfg.only_tools(tool_list);
        }

        cfg
    };

    info!("Starting downstream MCP server: {:?}", args.command);

    let mut cmd = Command::new(&args.command[0]);
    if args.command.len() > 1 {
        cmd.args(&args.command[1..]);
    }

    let transport = TokioChildProcess::new(cmd)?;

    info!("Connecting to downstream server...");
    let downstream = ().serve(transport).await?;

    info!("Starting proxy server on stdio...");
    let proxy = CodeModeProxy::new(downstream, config);

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let server_transport = (stdin, stdout);

    let service = proxy.serve(server_transport).await?;

    info!("Proxy server running. Waiting for shutdown...");
    service.waiting().await?;

    Ok(())
}
