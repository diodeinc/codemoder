use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddParams {
    #[schemars(description = "First number")]
    pub a: i64,
    #[schemars(description = "Second number")]
    pub b: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MultiplyParams {
    #[schemars(description = "First number")]
    pub a: i64,
    #[schemars(description = "Second number")]
    pub b: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EchoParams {
    #[schemars(description = "Message to echo back")]
    pub message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetItemsParams {}

#[derive(Clone)]
pub struct MockServer {
    tool_router: ToolRouter<Self>,
}

impl Default for MockServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl MockServer {
    #[tool(description = "Add two numbers together")]
    async fn add(
        &self,
        Parameters(params): Parameters<AddParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = params.a + params.b;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({"result": result}).to_string(),
        )]))
    }

    #[tool(description = "Multiply two numbers together")]
    async fn multiply(
        &self,
        Parameters(params): Parameters<MultiplyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = params.a * params.b;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({"result": result}).to_string(),
        )]))
    }

    #[tool(description = "Echo a message back")]
    async fn echo(
        &self,
        Parameters(params): Parameters<EchoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({"echo": params.message}).to_string(),
        )]))
    }

    #[tool(description = "Get a list of items")]
    async fn get_items(
        &self,
        Parameters(_params): Parameters<GetItemsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let items = vec![
            serde_json::json!({"id": "item-1", "name": "First Item", "value": 10}),
            serde_json::json!({"id": "item-2", "name": "Second Item", "value": 20}),
            serde_json::json!({"id": "item-3", "name": "Third Item", "value": 30}),
        ];
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({"items": items}).to_string(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for MockServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "mock-mcp-server".into(),
                version: "1.0.0".into(),
                title: Some("Mock MCP Server".to_string()),
                icons: None,
                website_url: None,
            },
            instructions: Some("A mock MCP server for testing".to_string()),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = MockServer::new();
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
