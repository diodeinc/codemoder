use crate::config::{CodeModeConfig, CodeModeExposure};
use crate::runtime::JsRuntime;
use crate::typescript::generate_typescript_interface;
use rmcp::ServerHandler;
use rmcp::model::*;
use rmcp::schemars::JsonSchema;
use rmcp::service::{RequestContext, RoleServer};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteCodeParams {
    #[schemars(
        description = "JavaScript code to execute. The code has access to a `tools` object with synchronous functions for each tool. The last expression is returned. IMPORTANT: Semicolons are required after statements, and object literals must be wrapped in parentheses: ({key: value});"
    )]
    #[allow(dead_code)]
    pub code: String,
}

pub struct CodeModeProxy {
    config: CodeModeConfig,
    downstream: Arc<Mutex<rmcp::service::RunningService<rmcp::service::RoleClient, ()>>>,
    cached_tools: RwLock<Vec<Tool>>,
    cached_ts_interface: RwLock<String>,
    runtime: Arc<Mutex<Option<JsRuntime>>>,
}

impl CodeModeProxy {
    pub fn new(
        downstream: rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
        config: CodeModeConfig,
    ) -> Self {
        Self {
            config,
            downstream: Arc::new(Mutex::new(downstream)),
            cached_tools: RwLock::new(Vec::new()),
            cached_ts_interface: RwLock::new(String::new()),
            runtime: Arc::new(Mutex::new(None)),
        }
    }

    async fn make_execute_tools_tool(&self) -> Tool {
        use rmcp::handler::server::common::schema_for_type;

        let ts_interface = self.cached_ts_interface.read().await.clone();
        let description = if ts_interface.is_empty() {
            self.config.tool_description.clone()
        } else {
            format!(
                "{}\n\n## Available Tools (synchronous)\n\n```typescript\n{}\n```\n\n## Notes\n\n- All tool calls are **synchronous** (no async/await needed)\n- Use `console.log(value)` to debug - logs are returned in the result",
                self.config.tool_description, ts_interface
            )
        };

        Tool {
            name: self.config.tool_name.clone().into(),
            description: Some(description.into()),
            input_schema: Arc::new(schema_for_type::<ExecuteCodeParams>()),
            title: None,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    fn filter_tools(&self, tools: Vec<Tool>) -> Vec<Tool> {
        match &self.config.include_tools {
            Some(include) => tools
                .into_iter()
                .filter(|t| include.iter().any(|name| name == t.name.as_ref()))
                .collect(),
            None => tools,
        }
    }

    pub async fn list_all_tools(&self) -> Result<Vec<Tool>, ErrorData> {
        let peer = self.downstream.lock().await;
        let inner_result = peer
            .peer()
            .list_tools(None)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Downstream error: {e}"), None))?;

        let inner_tools = self.filter_tools(inner_result.tools);

        {
            let mut cached = self.cached_tools.write().await;
            *cached = inner_tools.clone();
        }
        {
            let mut cached = self.cached_ts_interface.write().await;
            *cached = generate_typescript_interface(&inner_tools, "tools");
        }

        let mut result_tools = match self.config.mode {
            CodeModeExposure::ReplaceTools => vec![],
            CodeModeExposure::Add => inner_tools,
        };

        result_tools.push(self.make_execute_tools_tool().await);
        Ok(result_tools)
    }

    pub async fn call_tool_direct(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, ErrorData> {
        let peer = self.downstream.lock().await;

        let request = rmcp::model::CallToolRequestParam {
            name: name.to_string().into(),
            arguments: args.as_object().cloned(),
        };

        let result = peer
            .peer()
            .call_tool(request)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Downstream error: {e}"), None))?;

        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        Ok(text)
    }

    pub async fn execute_code_direct(&self, code: &str) -> Result<serde_json::Value, ErrorData> {
        let result = self.execute_code(code).await?;
        if result.is_error {
            return Err(ErrorData::internal_error(
                result.error_message.unwrap_or_default(),
                None,
            ));
        }
        Ok(result.value)
    }

    async fn ensure_tools_cached(&self) -> Result<(), ErrorData> {
        let cached = self.cached_tools.read().await;
        if !cached.is_empty() {
            return Ok(());
        }
        drop(cached);

        let peer = self.downstream.lock().await;
        let inner_result = peer
            .peer()
            .list_tools(None)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Downstream error: {e}"), None))?;

        let inner_tools = self.filter_tools(inner_result.tools);

        {
            let mut cached = self.cached_tools.write().await;
            *cached = inner_tools.clone();
        }
        {
            let mut cached = self.cached_ts_interface.write().await;
            *cached = generate_typescript_interface(&inner_tools, "tools");
        }

        Ok(())
    }

    async fn execute_code(&self, code: &str) -> Result<crate::runtime::ExecutionResult, ErrorData> {
        self.ensure_tools_cached().await?;

        let tools = self.cached_tools.read().await.clone();
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();

        let full_code = code.to_string();

        let mut runtime_guard = self.runtime.lock().await;
        if runtime_guard.is_none() {
            *runtime_guard = Some(
                JsRuntime::new()
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            );
        }

        let runtime = runtime_guard.as_ref().unwrap();
        let downstream = self.downstream.clone();

        runtime
            .execute_with_tools(&full_code, &tool_names, downstream)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Code execution failed: {e}"), None))
    }
}

impl ServerHandler for CodeModeProxy {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "code-mode-proxy".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Code Mode MCP Proxy".to_string()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "This proxy adds code-mode capability. Use the execute_tools tool to write JavaScript that calls multiple tools.".to_string()
            ),
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let downstream = self.downstream.lock().await;
        let peer = downstream.peer();

        let inner_result = peer
            .list_tools(request)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Downstream error: {e}"), None))?;

        let inner_tools = self.filter_tools(inner_result.tools);

        {
            let mut cached = self.cached_tools.write().await;
            *cached = inner_tools.clone();
        }
        {
            let mut cached = self.cached_ts_interface.write().await;
            *cached = generate_typescript_interface(&inner_tools, "tools");
        }

        let mut result_tools = match self.config.mode {
            CodeModeExposure::ReplaceTools => vec![],
            CodeModeExposure::Add => inner_tools,
        };

        result_tools.push(self.make_execute_tools_tool().await);

        Ok(ListToolsResult {
            tools: result_tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if request.name.as_ref() == self.config.tool_name {
            let code = request
                .arguments
                .as_ref()
                .and_then(|args| args.get("code"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| ErrorData::invalid_params("Missing 'code' parameter", None))?;

            let result = self.execute_code(code).await?;

            // Build the response content
            let response_value = if result.logs.is_empty() {
                result.value.clone()
            } else {
                serde_json::json!({
                    "result": result.value,
                    "logs": result.logs
                })
            };

            let content = if result.is_error {
                // Include error message in the content
                let error_response = serde_json::json!({
                    "error": result.error_message.as_deref().unwrap_or("Unknown error"),
                    "logs": result.logs
                });
                Content::text(serde_json::to_string_pretty(&error_response).unwrap_or_default())
            } else {
                Content::text(
                    serde_json::to_string_pretty(&response_value)
                        .unwrap_or_else(|_| response_value.to_string()),
                )
            };

            return Ok(CallToolResult {
                content: vec![content],
                is_error: Some(result.is_error),
                structured_content: None,
                meta: None,
            });
        }

        let downstream = self.downstream.lock().await;
        let peer = downstream.peer();

        peer.call_tool(request)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Downstream error: {e}"), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodeModeConfig;

    fn make_test_tool(name: &str) -> Tool {
        let name = name.to_string();
        let desc = format!("Test tool: {name}");
        Tool {
            name: name.into(),
            description: Some(desc.into()),
            input_schema: Arc::new(
                serde_json::json!({"type": "object", "properties": {}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            title: None,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn test_filter_tools_empty_config() {
        let tools = vec![make_test_tool("tool1"), make_test_tool("tool2")];

        let config = CodeModeConfig::default();
        let filtered = match &config.include_tools {
            Some(include) => tools
                .into_iter()
                .filter(|t| include.iter().any(|name| name == t.name.as_ref()))
                .collect(),
            None => tools,
        };

        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_tools_with_selection() {
        let tools = vec![
            make_test_tool("tool1"),
            make_test_tool("tool2"),
            make_test_tool("tool3"),
        ];

        let config =
            CodeModeConfig::new().only_tools(vec!["tool1".to_string(), "tool3".to_string()]);
        let filtered: Vec<Tool> = match &config.include_tools {
            Some(include) => tools
                .into_iter()
                .filter(|t| include.iter().any(|name| name == t.name.as_ref()))
                .collect(),
            None => tools,
        };

        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|t| t.name == "tool1"));
        assert!(filtered.iter().any(|t| t.name == "tool3"));
    }
}
