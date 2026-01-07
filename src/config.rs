use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CodeModeExposure {
    ReplaceTools,
    #[default]
    Add,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeModeConfig {
    pub mode: CodeModeExposure,
    pub tool_name: String,
    pub tool_description: String,
    pub include_tools: Option<Vec<String>>,
}

impl Default for CodeModeConfig {
    fn default() -> Self {
        Self {
            mode: CodeModeExposure::default(),
            tool_name: "execute_tools".to_string(),
            tool_description: r#"Execute JavaScript code with access to MCP tools. The code has access to a `tools` object with synchronous functions for each tool. The last expression is returned as the result. Use `console.log()` to debug.

## Important syntax rules

1. **Semicolons are required** after statements (strict ECMAScript parsing)
2. **Object literals must be wrapped in parentheses** when used as the final expression: `({key: value})`
3. The last expression in the code is automatically returned

## Examples

Query and process data:
```javascript
var items = tools.get_items({});
var total = 0;
for (var i = 0; i < items.length; i++) {
    total += items[i].value;
}
total;
```

Return an object (note the parentheses):
```javascript
var a = tools.add({a: 5, b: 3});
var b = tools.multiply({a: a.result, b: 2});
({sum: a.result, product: b.result});
```

Filter and transform:
```javascript
var items = tools.get_items({}).filter(function(x) { return x.value > 10; });
items.map(function(x) { return x.name; });
```"#
                .to_string(),
            include_tools: None,
        }
    }
}

impl CodeModeConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_tools(mut self) -> Self {
        self.mode = CodeModeExposure::ReplaceTools;
        self
    }

    pub fn add(mut self) -> Self {
        self.mode = CodeModeExposure::Add;
        self
    }

    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = name.into();
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.tool_description = desc.into();
        self
    }

    pub fn only_tools(mut self, tools: Vec<String>) -> Self {
        self.include_tools = Some(tools);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CodeModeConfig::default();
        assert_eq!(config.tool_name, "execute_tools");
        assert!(matches!(config.mode, CodeModeExposure::Add));
        assert!(config.include_tools.is_none());
    }

    #[test]
    fn test_builder_pattern() {
        let config = CodeModeConfig::new()
            .replace_tools()
            .with_tool_name("run_script")
            .only_tools(vec!["tool1".to_string(), "tool2".to_string()]);

        assert_eq!(config.tool_name, "run_script");
        assert!(matches!(config.mode, CodeModeExposure::ReplaceTools));
        assert_eq!(
            config.include_tools,
            Some(vec!["tool1".to_string(), "tool2".to_string()])
        );
    }
}
