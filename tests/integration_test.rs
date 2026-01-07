use rmcp::model::CallToolRequestParam;
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ServiceExt, transport::TokioChildProcess};
use std::path::PathBuf;
use tokio::process::Command;

type Client = RunningService<RoleClient, ()>;

fn get_codemoder_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codemoder"))
}

fn get_mock_server_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock-mcp-server"))
}

async fn setup_client() -> Client {
    let codemoder_path = get_codemoder_path();
    let mock_server_path = get_mock_server_path();

    let mut cmd = Command::new(&codemoder_path);
    cmd.arg(mock_server_path);

    let transport = TokioChildProcess::new(cmd).expect("Failed to create transport");
    ().serve(transport)
        .await
        .expect("Failed to connect to codemoder")
}

async fn setup_client_with_args(args: &[&str]) -> Client {
    let codemoder_path = get_codemoder_path();
    let mock_server_path = get_mock_server_path();

    let mut cmd = Command::new(&codemoder_path);
    cmd.args(args);
    cmd.arg("--");
    cmd.arg(mock_server_path);

    let transport = TokioChildProcess::new(cmd).expect("Failed to create transport");
    ().serve(transport)
        .await
        .expect("Failed to connect to codemoder")
}

async fn call_tool(client: &Client, name: &str, args: serde_json::Value) -> String {
    let name_owned: String = name.to_string();
    let result = client
        .peer()
        .call_tool(CallToolRequestParam {
            name: name_owned.into(),
            arguments: Some(args.as_object().unwrap().clone()),
        })
        .await
        .expect("Failed to call tool");

    result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_tools_includes_execute_code() {
    let client = setup_client().await;

    let result = client.peer().list_all_tools().await.unwrap();

    let tool_names: Vec<_> = result.iter().map(|t| t.name.as_ref()).collect();
    assert!(tool_names.contains(&"execute_tools"));
    assert!(tool_names.contains(&"add"));
    assert!(tool_names.contains(&"multiply"));
    assert!(tool_names.contains(&"echo"));
    assert!(tool_names.contains(&"get_items"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_proxy_tool_call() {
    let client = setup_client().await;

    let result = call_tool(&client, "add", serde_json::json!({"a": 5, "b": 3})).await;
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["result"], 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_execute_code_simple() {
    let client = setup_client().await;

    let result = call_tool(
        &client,
        "execute_tools",
        serde_json::json!({"code": "1 + 2"}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_execute_code_calls_tool() {
    let client = setup_client().await;

    let code = r#"
        var result = tools.add({a: 10, b: 20});
        result
    "#;

    let result = call_tool(&client, "execute_tools", serde_json::json!({"code": code})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["result"].as_f64().unwrap() as i64, 30);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_execute_code_multiple_tool_calls() {
    let client = setup_client().await;

    let code = r#"
        var sum = tools.add({a: 5, b: 5});
        var product = tools.multiply({a: sum.result, b: 3});
        ({sum: sum.result, product: product.result})
    "#;

    let result = call_tool(&client, "execute_tools", serde_json::json!({"code": code})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["sum"].as_f64().unwrap() as i64, 10);
    assert_eq!(json["product"].as_f64().unwrap() as i64, 30);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_execute_code_with_loop() {
    let client = setup_client().await;

    let code = r#"
        var items = tools.get_items({});
        var total = 0;
        for (var i = 0; i < items.items.length; i++) {
            total += items.items[i].value;
        }
        ({itemCount: items.items.length, totalValue: total})
    "#;

    let result = call_tool(&client, "execute_tools", serde_json::json!({"code": code})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["itemCount"].as_f64().unwrap() as i64, 3);
    assert_eq!(json["totalValue"].as_f64().unwrap() as i64, 60);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_replace_mode() {
    let client = setup_client_with_args(&["--mode", "replace"]).await;

    let result = client.peer().list_all_tools().await.unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "execute_tools");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_echo_tool() {
    let client = setup_client().await;

    let code = r#"
        tools.echo({message: "Hello, World!"})
    "#;

    let result = call_tool(&client, "execute_tools", serde_json::json!({"code": code})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["echo"], "Hello, World!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_console_log() {
    let client = setup_client().await;

    let code = r#"
        console.log("debug message");
        "done"
    "#;

    let result = call_tool(&client, "execute_tools", serde_json::json!({"code": code})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["result"], "done");
    assert!(
        json["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "debug message")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tool_error_propagates() {
    let client = setup_client().await;

    let code = r#"
        tools.add({});
    "#;

    let result = client
        .peer()
        .call_tool(CallToolRequestParam {
            name: "execute_tools".into(),
            arguments: Some(
                serde_json::json!({"code": code})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        })
        .await;

    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert!(call_result.is_error.unwrap_or(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tool_includes_typescript_interface() {
    let client = setup_client().await;

    let result = client.peer().list_all_tools().await.unwrap();

    let execute_tools_tool = result.iter().find(|t| t.name == "execute_tools").unwrap();
    let description = execute_tools_tool.description.as_ref().unwrap();

    assert!(description.contains("declare namespace tools"));
    assert!(description.contains("function add"));
    assert!(description.contains("function multiply"));
    assert!(description.contains("console.log"));
}
