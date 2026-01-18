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

/// Convert a JSON value to Content items.
/// Recognizes image objects with {type: "image", data: "...", mimeType: "..."} format
/// and converts them back to proper Content::image.
fn json_to_content(value: &serde_json::Value) -> Vec<Content> {
    // Check if it's an image object
    if let Some(obj) = value.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("image") {
            if let (Some(data), Some(mime_type)) = (
                obj.get("data").and_then(|v| v.as_str()),
                obj.get("mimeType").and_then(|v| v.as_str()),
            ) {
                return vec![Content::image(data, mime_type)];
            }
        }
        // Check if it has a "result" field (from logs wrapper)
        if let Some(result) = obj.get("result") {
            let mut content = json_to_content(result);
            if let Some(logs) = obj.get("logs") {
                if let Some(logs_arr) = logs.as_array() {
                    if !logs_arr.is_empty() {
                        content.push(Content::text(format!(
                            "Logs:\n{}",
                            logs_arr
                                .iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        )));
                    }
                }
            }
            return content;
        }
    }

    // Check if it's an array that might contain images
    if let Some(arr) = value.as_array() {
        let mut content = Vec::new();
        for item in arr {
            content.extend(json_to_content(item));
        }
        if !content.is_empty() {
            return content;
        }
    }

    // Default: convert to text
    vec![Content::text(
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    )]
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteCodeParams {
    #[schemars(
        description = "JavaScript code to execute. The code has access to a `tools` object with synchronous functions for each tool. The last expression is returned. IMPORTANT: Semicolons are required after statements, and object literals must be wrapped in parentheses: ({key: value});"
    )]
    #[allow(dead_code)]
    pub code: String,
}

/// A wrapper that adds code-mode capability to any ServerHandler.
///
/// This wraps an existing MCP server and adds an `execute_tools` tool that
/// allows executing JavaScript code with access to all the wrapped server's tools.
pub struct CodeModeWrapper<H: ServerHandler + Send + Sync + 'static> {
    config: CodeModeConfig,
    inner: Arc<H>,
    cached_tools: RwLock<Vec<Tool>>,
    cached_ts_interface: RwLock<String>,
    runtime: Arc<Mutex<Option<JsRuntime>>>,
}

impl<H: ServerHandler + Send + Sync + 'static> CodeModeWrapper<H> {
    pub fn new(inner: H, config: CodeModeConfig) -> Self {
        Self {
            config,
            inner: Arc::new(inner),
            cached_tools: RwLock::new(Vec::new()),
            cached_ts_interface: RwLock::new(String::new()),
            runtime: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_default_config(inner: H) -> Self {
        Self::new(inner, CodeModeConfig::default())
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

    async fn ensure_tools_cached(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let cached = self.cached_tools.read().await;
        if !cached.is_empty() {
            return Ok(());
        }
        drop(cached);

        let inner_result = self.inner.list_tools(None, context.clone()).await?;
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

    async fn execute_code(
        &self,
        code: &str,
        context: &RequestContext<RoleServer>,
    ) -> Result<crate::runtime::ExecutionResult, ErrorData> {
        self.ensure_tools_cached(context).await?;

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
        let inner = self.inner.clone();
        let context = context.clone();

        runtime
            .execute_with_handler(&full_code, &tool_names, inner, context)
            .await
            .map_err(|e| ErrorData::internal_error(format!("Code execution failed: {e}"), None))
    }
}

impl<H: ServerHandler + Send + Sync + 'static> ServerHandler for CodeModeWrapper<H> {
    fn get_info(&self) -> ServerInfo {
        let mut info = self.inner.get_info();
        info.instructions = Some(format!(
            "{}\n\nThis server has code-mode enabled. Use the {} tool to write JavaScript that calls multiple tools.",
            info.instructions.unwrap_or_default(),
            self.config.tool_name
        ));
        info
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let inner_result = self.inner.list_tools(request, context).await?;
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if request.name.as_ref() == self.config.tool_name {
            let code = request
                .arguments
                .as_ref()
                .and_then(|args| args.get("code"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| ErrorData::invalid_params("Missing 'code' parameter", None))?;

            let result = self.execute_code(code, &context).await?;

            let response_value = if result.logs.is_empty() {
                result.value.clone()
            } else {
                serde_json::json!({
                    "result": result.value,
                    "logs": result.logs
                })
            };

            let content = if result.is_error {
                let error_response = serde_json::json!({
                    "error": result.error_message.as_deref().unwrap_or("Unknown error"),
                    "logs": result.logs
                });
                vec![Content::text(
                    serde_json::to_string_pretty(&error_response).unwrap_or_default(),
                )]
            } else {
                json_to_content(&response_value)
            };

            return Ok(CallToolResult {
                content,
                is_error: Some(result.is_error),
                structured_content: None,
                meta: None,
            });
        }

        self.inner.call_tool(request, context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_to_content_text() {
        let value = serde_json::json!("hello world");
        let content = json_to_content(&value);
        assert_eq!(content.len(), 1);
        assert!(content[0].as_text().is_some());
    }

    #[test]
    fn test_json_to_content_image() {
        let value = serde_json::json!({
            "type": "image",
            "data": "SGVsbG8=",
            "mimeType": "image/png"
        });
        let content = json_to_content(&value);
        assert_eq!(content.len(), 1);
        let img = content[0].as_image().expect("should be image content");
        assert_eq!(img.data, "SGVsbG8=");
        assert_eq!(img.mime_type, "image/png");
    }

    #[test]
    fn test_json_to_content_array_with_images() {
        let value = serde_json::json!([
            {"type": "image", "data": "abc123", "mimeType": "image/png"},
            "some text"
        ]);
        let content = json_to_content(&value);
        assert_eq!(content.len(), 2);
        assert!(content[0].as_image().is_some());
        assert!(content[1].as_text().is_some());
    }

    #[test]
    fn test_json_to_content_result_with_logs() {
        let value = serde_json::json!({
            "result": {"type": "image", "data": "abc123", "mimeType": "image/png"},
            "logs": ["log1", "log2"]
        });
        let content = json_to_content(&value);
        assert_eq!(content.len(), 2);
        assert!(content[0].as_image().is_some());
        let text = content[1].as_text().expect("should have logs text");
        assert!(text.text.contains("log1"));
        assert!(text.text.contains("log2"));
    }
}
