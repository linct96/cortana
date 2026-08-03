use super::openai_responses::{
    alias_for_tool, insert_if_some, reasoning_text, tool_output_text, ResponseStream, ToolMeta,
};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, io::Read};

pub(super) fn handle_event<R: Read>(stream: &mut ResponseStream<R>, value: &Value) {
    if let Some(error) = value.get("error") {
        stream.fail(
            error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("upstream_error"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("上游流错误。"),
        );
        return;
    }
    if let Some(usage) = value.get("usage").filter(|value| !value.is_null()) {
        stream.set_usage(
            usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
    }
    for choice in value
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let choice_index = choice.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
            stream.reasoning_delta(choice_index, text);
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            stream.text_delta(choice_index, text);
        }
        for tool in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = tool.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(alias) = tool.pointer("/function/name").and_then(Value::as_str) {
                stream.start_tool(
                    index,
                    tool.get("id").and_then(Value::as_str).unwrap_or_default(),
                    alias,
                );
            }
            if let Some(arguments) = tool.pointer("/function/arguments").and_then(Value::as_str) {
                stream.tool_delta(index, arguments);
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            stream.set_stop_reason(Some(reason));
        }
    }
}

pub(super) fn encode_body(
    request: &Map<String, Value>,
    model: &str,
    instructions: &str,
    input: &[Value],
    tools: Vec<Value>,
    tool_map: &HashMap<String, ToolMeta>,
    tool_choice: Option<Value>,
) -> Result<Value, String> {
    let mut body = json!({
        "model": model,
        "messages": encode_chat_messages(instructions, input, tool_map)?,
        "stream": true,
        "stream_options": { "include_usage": true },
        "tools": tools,
    });
    insert_if_some(&mut body, "tool_choice", tool_choice);
    if let Some(value) = request.get("parallel_tool_calls").and_then(Value::as_bool) {
        body["parallel_tool_calls"] = json!(value);
    }
    Ok(body)
}

fn encode_chat_messages(
    instructions: &str,
    input: &[Value],
    tools: &HashMap<String, ToolMeta>,
) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    if !instructions.is_empty() {
        messages.push(json!({"role":"system","content":instructions}));
    }
    for item in input {
        match item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message")
        {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                messages.push(json!({"role":role,"content":text_parts(item.get("content"), true)}));
            }
            "function_call" | "custom_tool_call" => {
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let alias = alias_for_tool(name, tools).unwrap_or(name);
                let arguments = item
                    .get("arguments")
                    .or_else(|| item.get("input"))
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let arguments =
                    if item.get("type").and_then(Value::as_str) == Some("custom_tool_call") {
                        serde_json::to_string(&json!({"input":arguments})).unwrap()
                    } else {
                        arguments.to_string()
                    };
                messages.push(json!({"role":"assistant","tool_calls":[{
                    "id": item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                    "type":"function","function":{"name":alias,"arguments":arguments}
                }]}));
            }
            "function_call_output" | "custom_tool_call_output" => {
                messages.push(json!({
                    "role":"tool",
                    "tool_call_id":item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                    "content":tool_output_text(item.get("output"))
                }));
                for image in tool_output_images(item.get("output")) {
                    messages.push(json!({"role":"user","content":[image]}));
                }
            }
            "reasoning" => {
                let summary = reasoning_text(item);
                if !summary.is_empty() {
                    messages.push(json!({"role":"assistant","reasoning_content":summary}));
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(messages)
}

fn text_parts(content: Option<&Value>, input: bool) -> Vec<Value> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("input_text" | "output_text") => part
                .get("text")
                .and_then(Value::as_str)
                .map(|text| json!({"type":"text","text":text})),
            Some("input_image") if input => part
                .get("image_url")
                .and_then(Value::as_str)
                .map(|url| json!({"type":"image_url","image_url":{"url":url}})),
            _ => None,
        })
        .collect()
}

fn tool_output_images(output: Option<&Value>) -> Vec<Value> {
    output
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| {
            (part.get("type").and_then(Value::as_str) == Some("input_image"))
                .then(|| part.get("image_url").and_then(Value::as_str))
                .flatten()
                .map(|url| json!({"type":"image_url","image_url":{"url":url}}))
        })
        .collect()
}
