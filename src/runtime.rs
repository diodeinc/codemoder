use anyhow::{Context, Result};
use rmcp::ServerHandler;
use rmcp::model::{CallToolRequestParam, CallToolResult};
use rmcp::service::{RequestContext, RoleServer};
use rquickjs::{AsyncContext, AsyncRuntime, Function, Object, Type, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type DownstreamClient = rmcp::service::RunningService<rmcp::service::RoleClient, ()>;

pub trait ToolCaller: Send + Sync + 'static {
    fn call_tool_blocking(
        &self,
        name: &str,
        args: Option<serde_json::Value>,
    ) -> Result<CallToolResult>;
}

pub struct DownstreamToolCaller {
    client: Arc<Mutex<DownstreamClient>>,
}

impl DownstreamToolCaller {
    pub fn new(client: Arc<Mutex<DownstreamClient>>) -> Self {
        Self { client }
    }
}

impl ToolCaller for DownstreamToolCaller {
    fn call_tool_blocking(
        &self,
        tool_name: &str,
        args: Option<serde_json::Value>,
    ) -> Result<CallToolResult> {
        use tokio::runtime::Handle;

        let tool_name = tool_name.to_string();
        let arguments = args.and_then(|v| v.as_object().cloned());
        let client = self.client.clone();

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let client = client.lock().await;
                let peer = client.peer();

                let request = CallToolRequestParam {
                    name: tool_name.into(),
                    arguments,
                };

                peer.call_tool(request)
                    .await
                    .map_err(|e| anyhow::anyhow!("Tool call failed: {e}"))
            })
        })
    }
}

pub struct HandlerToolCaller<H: ServerHandler + Send + Sync + 'static> {
    handler: Arc<H>,
    context: RequestContext<RoleServer>,
}

impl<H: ServerHandler + Send + Sync + 'static> HandlerToolCaller<H> {
    pub fn new(handler: Arc<H>, context: RequestContext<RoleServer>) -> Self {
        Self { handler, context }
    }
}

impl<H: ServerHandler + Send + Sync + 'static> ToolCaller for HandlerToolCaller<H> {
    fn call_tool_blocking(
        &self,
        tool_name: &str,
        args: Option<serde_json::Value>,
    ) -> Result<CallToolResult> {
        use tokio::runtime::Handle;

        let tool_name = tool_name.to_string();
        let arguments = args.and_then(|v| v.as_object().cloned());
        let handler = self.handler.clone();
        let context = self.context.clone();

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let request = CallToolRequestParam {
                    name: tool_name.into(),
                    arguments,
                };

                handler
                    .call_tool(request, context)
                    .await
                    .map_err(|e| anyhow::anyhow!("Tool call failed: {e:?}"))
            })
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionResult {
    pub value: serde_json::Value,
    pub logs: Vec<String>,
    pub is_error: bool,
    pub error_message: Option<String>,
}

pub struct JsRuntime {
    runtime: AsyncRuntime,
}

impl JsRuntime {
    pub async fn new() -> Result<Self> {
        let runtime = AsyncRuntime::new()?;
        Ok(Self { runtime })
    }

    pub async fn execute(&self, code: &str) -> Result<serde_json::Value> {
        let code = code.to_string();
        let context = AsyncContext::full(&self.runtime).await?;

        context
            .with(|ctx| {
                let result: Value = ctx.eval(code.as_bytes().to_vec())?;
                value_to_json(&result)
            })
            .await
    }

    pub async fn execute_with_tools(
        &self,
        code: &str,
        tool_names: &[String],
        downstream: Arc<Mutex<DownstreamClient>>,
    ) -> Result<ExecutionResult> {
        let caller = Arc::new(DownstreamToolCaller::new(downstream));
        self.execute_with_caller(code, tool_names, caller).await
    }

    pub async fn execute_with_handler<H: ServerHandler + Send + Sync + 'static>(
        &self,
        code: &str,
        tool_names: &[String],
        handler: Arc<H>,
        context: RequestContext<RoleServer>,
    ) -> Result<ExecutionResult> {
        let caller = Arc::new(HandlerToolCaller::new(handler, context));
        self.execute_with_caller(code, tool_names, caller).await
    }

    pub async fn execute_with_caller<C: ToolCaller>(
        &self,
        code: &str,
        tool_names: &[String],
        caller: Arc<C>,
    ) -> Result<ExecutionResult> {
        let code = code.to_string();
        let tool_names = tool_names.to_vec();
        let logs: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        let context = AsyncContext::full(&self.runtime).await?;

        context
            .with(move |ctx| {
                let globals = ctx.globals();

                // Set up console.log
                let console = Object::new(ctx.clone())?;
                let logs_for_closure = logs_clone.clone();
                let log_fn = Function::new(ctx.clone(), move |args: String| {
                    if let Ok(mut logs) = logs_for_closure.lock() {
                        logs.push(args);
                    }
                })?;
                console.set("log", log_fn)?;
                globals.set("console", console)?;

                // Set up __stringify helper for console.log
                let stringify_setup = r#"
                    var __original_console_log = console.log;
                    console.log = function() {
                        var parts = [];
                        for (var i = 0; i < arguments.length; i++) {
                            var arg = arguments[i];
                            if (typeof arg === 'object') {
                                parts.push(JSON.stringify(arg));
                            } else {
                                parts.push(String(arg));
                            }
                        }
                        __original_console_log(parts.join(' '));
                    };
                "#;
                let _: Value = ctx.eval(stringify_setup.as_bytes().to_vec())?;

                let raw_tools = Object::new(ctx.clone())?;

                for tool_name in &tool_names {
                    let name = tool_name.clone();
                    let caller_clone = caller.clone();

                    let func = Function::new(ctx.clone(), move |args: String| {
                        let tool_name = name.clone();
                        let caller = caller_clone.clone();

                        let args_value: Option<serde_json::Value> = serde_json::from_str(&args).ok();
                        let result = caller.call_tool_blocking(&tool_name, args_value);

                        match result {
                            Ok(call_result) => format_call_result(&call_result),
                            Err(e) => format!("{{\"error\": \"{e}\"}}"),
                        }
                    })?;

                    raw_tools.set(tool_name.as_str(), func)?;
                }

                globals.set("__raw_tools", raw_tools)?;

                let tool_names_json = serde_json::to_string(&tool_names).unwrap_or("[]".to_string());
                let tool_wrapper_code = format!(r#"
                    var tools = {{}};
                    var __tool_names = {tool_names_json};
                    for (var i = 0; i < __tool_names.length; i++) {{
                        (function(toolName) {{
                            tools[toolName] = function(args) {{
                                var jsonArgs = JSON.stringify(args || {{}});
                                var resultStr = __raw_tools[toolName](jsonArgs);
                                var result;
                                try {{
                                    result = JSON.parse(resultStr);
                                }} catch (e) {{
                                    result = resultStr;
                                }}
                                // If result contains an error field, throw it as an exception
                                if (result && typeof result === 'object' && result.error) {{
                                    throw new Error('Tool ' + toolName + ' failed: ' + result.error);
                                }}
                                return result;
                            }};
                        }})(__tool_names[i]);
                    }}
                "#);
                let wrapper_result: Result<Value, _> = ctx.eval(tool_wrapper_code.as_bytes().to_vec());
                if let Err(e) = wrapper_result {
                    return Err(anyhow::anyhow!("Tool wrapper setup failed: {e:?}"));
                }

                let code_result: Result<Value, _> = ctx.eval(code.as_bytes().to_vec());
                match code_result {
                    Ok(result) => Ok((value_to_json(&result)?, None)),
                    Err(_e) => {
                        let error_msg = if let Some(exc) = ctx.catch().as_exception() {
                            exc.message().unwrap_or_default().to_string()
                        } else {
                            "Unknown JavaScript error".to_string()
                        };
                        // Return the error as a successful result with is_error=true
                        Ok((serde_json::Value::Null, Some(error_msg)))
                    }
                }
            })
            .await
            .map(|(value, error)| {
                let captured_logs = logs.lock().map(|l| l.clone()).unwrap_or_default();
                ExecutionResult {
                    value,
                    logs: captured_logs,
                    is_error: error.is_some(),
                    error_message: error,
                }
            })
    }
}

fn format_call_result(result: &CallToolResult) -> String {
    let contents: Vec<serde_json::Value> = result
        .content
        .iter()
        .map(|c| {
            if let Some(text) = c.as_text() {
                serde_json::Value::String(text.text.clone())
            } else if let Some(image) = c.as_image() {
                serde_json::json!({
                    "type": "image",
                    "data": image.data,
                    "mimeType": image.mime_type
                })
            } else {
                serde_json::Value::Null
            }
        })
        .collect();

    if contents.len() == 1
        && let Some(s) = contents[0].as_str()
    {
        return s.to_string();
    }

    serde_json::to_string(&contents).unwrap_or_else(|_| "[]".to_string())
}

fn value_to_json(value: &Value) -> Result<serde_json::Value> {
    let type_of = value.type_of();

    match type_of {
        Type::Undefined | Type::Null => Ok(serde_json::Value::Null),
        Type::Bool => {
            let b = value.as_bool().unwrap_or(false);
            Ok(serde_json::Value::Bool(b))
        }
        Type::Int => {
            let i = value.as_int().unwrap_or(0);
            Ok(serde_json::Value::Number(i.into()))
        }
        Type::Float => {
            let f = value.as_float().unwrap_or(0.0);
            Ok(serde_json::json!(f))
        }
        Type::String => {
            let s = value
                .as_string()
                .context("Expected string")?
                .to_string()
                .context("Failed to convert JS string")?;
            Ok(serde_json::Value::String(s))
        }
        Type::Array => {
            let arr = value.as_array().context("Expected array")?;
            let items: Result<Vec<serde_json::Value>> = arr
                .iter()
                .map(|item| {
                    let item = item?;
                    value_to_json(&item)
                })
                .collect();
            Ok(serde_json::Value::Array(items?))
        }
        Type::Object => {
            let obj = value.as_object().context("Expected object")?;
            let mut map = serde_json::Map::new();
            for key in obj.keys::<String>() {
                let key = key?;
                let val: Value = obj.get(&key)?;
                map.insert(key, value_to_json(&val)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Ok(serde_json::Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_js_execution() {
        let runtime = JsRuntime::new().await.unwrap();
        let result = runtime.execute("1 + 2").await.unwrap();
        assert_eq!(result, serde_json::json!(3));
    }

    #[tokio::test]
    async fn test_execute_returns_json() {
        let runtime = JsRuntime::new().await.unwrap();
        let result = runtime.execute("1 + 2").await.unwrap();
        assert_eq!(result, serde_json::json!(3));
    }

    #[tokio::test]
    async fn test_js_object_return() {
        let runtime = JsRuntime::new().await.unwrap();
        let result = runtime
            .execute(r#"({ name: "test", value: 42 })"#)
            .await
            .unwrap();
        assert_eq!(result["name"], "test");
        assert_eq!(result["value"], 42);
    }

    #[tokio::test]
    async fn test_js_array_return() {
        let runtime = JsRuntime::new().await.unwrap();
        let result = runtime.execute("[1, 2, 3]").await.unwrap();
        assert_eq!(result, serde_json::json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn test_js_string_return() {
        let runtime = JsRuntime::new().await.unwrap();
        let result = runtime.execute(r#""hello world""#).await.unwrap();
        assert_eq!(result, serde_json::json!("hello world"));
    }

    #[test]
    fn test_format_call_result_with_text() {
        use rmcp::model::{CallToolResult, Content};

        let result = CallToolResult::success(vec![Content::text("hello")]);
        let formatted = format_call_result(&result);
        assert_eq!(formatted, "hello");
    }

    #[test]
    fn test_format_call_result_with_image() {
        use rmcp::model::{CallToolResult, Content};

        let result = CallToolResult::success(vec![Content::image("SGVsbG8=", "image/png")]);
        let formatted = format_call_result(&result);
        let parsed: serde_json::Value = serde_json::from_str(&formatted).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image");
        assert_eq!(arr[0]["data"], "SGVsbG8=");
        assert_eq!(arr[0]["mimeType"], "image/png");
    }

    #[test]
    fn test_format_call_result_with_mixed_content() {
        use rmcp::model::{CallToolResult, Content};

        let result = CallToolResult::success(vec![
            Content::text("description"),
            Content::image("SGVsbG8=", "image/png"),
        ]);
        let formatted = format_call_result(&result);
        let parsed: serde_json::Value = serde_json::from_str(&formatted).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "description");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["data"], "SGVsbG8=");
        assert_eq!(arr[1]["mimeType"], "image/png");
    }
}
