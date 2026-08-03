use super::openai_responses::{
    alias_for_tool, decode_signature, insert_if_some, reasoning_text, tool_output_text,
    ResponseStream, ToolMeta,
};
use serde_json::{json, Value};
use std::{collections::HashMap, io::Read};

pub(super) fn handle_event<R: Read>(stream: &mut ResponseStream<R>, value: &Value) {
    let event = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    match event {
        "message_start" => {
            if let Some(usage) = value.pointer("/message/usage") {
                merge_usage(stream, usage);
            }
        }
        "content_block_start" => {
            match value.pointer("/content_block/type").and_then(Value::as_str) {
                Some("text") => stream.start_text(index),
                Some("thinking") => stream.reasoning_delta(
                    index,
                    value
                        .pointer("/content_block/thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                Some("tool_use") => stream.start_tool(
                    index,
                    value
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    value
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                Some(other) => stream.fail(
                    "unsupported_feature",
                    &format!("不支持 Anthropic 内容块 {other}。"),
                ),
                None => {}
            }
        }
        "content_block_delta" => match value.pointer("/delta/type").and_then(Value::as_str) {
            Some("text_delta") => stream.text_delta(
                index,
                value
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            Some("thinking_delta") => stream.reasoning_delta(
                index,
                value
                    .pointer("/delta/thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            Some("signature_delta") => stream.append_reasoning_signature(
                index,
                value
                    .pointer("/delta/signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            Some("input_json_delta") => stream.tool_delta(
                index,
                value
                    .pointer("/delta/partial_json")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            _ => {}
        },
        "content_block_stop" => {
            stream.end_text(index);
            stream.end_reasoning(index);
            stream.end_tool(index);
        }
        "message_delta" => {
            stream.set_stop_reason(value.pointer("/delta/stop_reason").and_then(Value::as_str));
            if let Some(usage) = value.get("usage") {
                merge_usage(stream, usage);
            }
        }
        "message_stop" => stream.finish(),
        "error" => stream.fail(
            "upstream_error",
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("上游流错误。"),
        ),
        "ping" => {}
        _ => eprintln!("Model gateway ignored unknown Anthropic event: {event}"),
    }
}

fn merge_usage<R: Read>(stream: &mut ResponseStream<R>, usage: &Value) {
    stream.merge_usage(
        usage.get("input_tokens").and_then(Value::as_u64),
        usage.get("output_tokens").and_then(Value::as_u64),
    );
}

pub(super) fn encode_body(
    model: &str,
    instructions: &str,
    input: &[Value],
    tools: Vec<Value>,
    tool_map: &HashMap<String, ToolMeta>,
    tool_choice: Option<Value>,
    max_tokens: i64,
) -> Result<Value, String> {
    let mut body = json!({
        "model": model,
        "system": instructions,
        "messages": encode_anthropic_messages(input, tool_map)?,
        "stream": true,
        "max_tokens": max_tokens,
        "tools": tools,
    });
    insert_if_some(&mut body, "tool_choice", tool_choice);
    Ok(body)
}

fn encode_anthropic_messages(
    input: &[Value],
    tools: &HashMap<String, ToolMeta>,
) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    for item in input {
        let (role, content) = match item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message")
        {
            "message" => (
                if item.get("role").and_then(Value::as_str) == Some("assistant") {
                    "assistant"
                } else {
                    "user"
                },
                anthropic_parts(item.get("content")),
            ),
            "function_call" | "custom_tool_call" => {
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let alias = alias_for_tool(name, tools).unwrap_or(name);
                let raw = item
                    .get("arguments")
                    .or_else(|| item.get("input"))
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let input = if item.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                {
                    json!({"input":raw})
                } else {
                    serde_json::from_str(raw).map_err(|_| "工具参数不是有效 JSON。".to_string())?
                };
                (
                    "assistant",
                    vec![json!({
                        "type":"tool_use",
                        "id":item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                        "name":alias,
                        "input":input
                    })],
                )
            }
            "function_call_output" | "custom_tool_call_output" => (
                "user",
                vec![json!({
                    "type":"tool_result",
                    "tool_use_id":item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                    "content":anthropic_tool_output(item.get("output")),
                    "is_error":item.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                })],
            ),
            "reasoning" => {
                let text = reasoning_text(item);
                let signature =
                    decode_signature(item.get("encrypted_content").and_then(Value::as_str));
                let mut block = json!({"type":"thinking","thinking":text});
                if let Some(signature) = signature {
                    block["signature"] = json!(signature);
                }
                ("assistant", vec![block])
            }
            _ => unreachable!(),
        };
        push_message(&mut messages, role, content);
    }
    Ok(messages)
}

fn anthropic_parts(content: Option<&Value>) -> Vec<Value> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("input_text" | "output_text") => part
                .get("text")
                .and_then(Value::as_str)
                .map(|text| json!({"type":"text","text":text})),
            Some("input_image") => part
                .get("image_url")
                .and_then(Value::as_str)
                .and_then(anthropic_image),
            _ => None,
        })
        .collect()
}

fn anthropic_image(url: &str) -> Option<Value> {
    if url.starts_with("https://") || url.starts_with("http://") {
        return Some(json!({"type":"image","source":{"type":"url","url":url}}));
    }
    let value = url.strip_prefix("data:")?;
    let (media_type, data) = value.split_once(";base64,")?;
    Some(json!({"type":"image","source":{
        "type":"base64","media_type":media_type,"data":data
    }}))
}

fn anthropic_tool_output(output: Option<&Value>) -> Value {
    match output {
        Some(Value::Array(parts)) => Value::Array(
            parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text") => part
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|text| json!({"type":"text","text":text})),
                    Some("input_image") => part
                        .get("image_url")
                        .and_then(Value::as_str)
                        .and_then(anthropic_image),
                    _ => None,
                })
                .collect(),
        ),
        _ => json!(tool_output_text(output)),
    }
}

fn push_message(messages: &mut Vec<Value>, role: &str, content: Vec<Value>) {
    if let Some(last) = messages.last_mut().filter(|last| last["role"] == role) {
        last["content"].as_array_mut().unwrap().extend(content);
    } else {
        messages.push(json!({"role":role,"content":content}));
    }
}
