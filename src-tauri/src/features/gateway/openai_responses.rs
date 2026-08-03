use super::{anthropic_messages, openai_chat, UpstreamProtocol};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::{self, BufRead, BufReader, Read},
};
use uuid::Uuid;

const MAX_UPSTREAM_SSE_LINE_BYTES: usize = 1024 * 1024;
const MAX_CONVERTED_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct ToolMeta {
    original_name: String,
    namespace: Option<String>,
    custom: bool,
}

pub(super) struct EncodedRequest {
    pub(super) body: Value,
    tools: HashMap<String, ToolMeta>,
}

pub(super) fn prepare_codex_oauth_request(
    input: Value,
    session_id: Option<&str>,
    installation_id: Option<&str>,
) -> Result<Value, String> {
    let object = input.as_object().expect("validated object");
    let mut output = codex_request_base(object)?;
    output.insert("tool_choice".to_string(), json!("auto"));
    output.insert("store".to_string(), json!(false));
    output.insert("stream".to_string(), json!(true));

    let mut include = object
        .get("include")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if object
        .get("reasoning")
        .and_then(Value::as_object)
        .is_some_and(|value| !value.is_empty())
        && !include
            .iter()
            .any(|value| value == "reasoning.encrypted_content")
    {
        include.push(json!("reasoning.encrypted_content"));
    }
    output.insert("include".to_string(), Value::Array(include));

    copy_codex_optional_fields(object, &mut output);
    insert_prompt_cache_key(object, &mut output, session_id);

    let mut metadata = object
        .get("client_metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(installation_id) = installation_id.filter(|value| !value.trim().is_empty()) {
        metadata
            .entry("x-codex-installation-id".to_string())
            .or_insert_with(|| json!(installation_id));
    }
    if !metadata.is_empty() {
        output.insert("client_metadata".to_string(), Value::Object(metadata));
    }
    Ok(Value::Object(output))
}

pub(super) fn prepare_codex_compact_request(
    input: Value,
    session_id: Option<&str>,
) -> Result<Value, String> {
    let object = input.as_object().expect("validated object");
    let mut output = codex_request_base(object)?;
    copy_codex_optional_fields(object, &mut output);
    insert_prompt_cache_key(object, &mut output, session_id);
    Ok(Value::Object(output))
}

fn codex_request_base(object: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let model = required_string(object, "model")?;
    let input = normalize_codex_input(
        object
            .get("input")
            .ok_or_else(|| "请求缺少 input。".to_string())?,
    )?;
    let tools = match object.get("tools") {
        Some(Value::Array(tools)) => tools.clone(),
        Some(_) => return Err("请求的 tools 必须是数组。".to_string()),
        None => Vec::new(),
    };
    let mut output = Map::from_iter([
        ("model".to_string(), json!(model)),
        ("input".to_string(), Value::Array(input)),
        ("tools".to_string(), Value::Array(tools)),
        (
            "parallel_tool_calls".to_string(),
            json!(object
                .get("parallel_tool_calls")
                .and_then(Value::as_bool)
                .unwrap_or(true)),
        ),
    ]);
    if let Some(Value::String(instructions)) = object.get("instructions") {
        if !instructions.is_empty() {
            output.insert("instructions".to_string(), json!(instructions));
        }
    }
    Ok(output)
}

fn normalize_codex_input(input: &Value) -> Result<Vec<Value>, String> {
    let mut input = match input {
        Value::Array(input) => input.clone(),
        Value::String(text) => vec![json!({"role":"user","content":text})],
        _ => return Err("请求的 input 必须是字符串或数组。".to_string()),
    };
    for item in &mut input {
        if item.get("type").and_then(Value::as_str) == Some("reasoning")
            && item.get("content").is_some_and(Value::is_null)
        {
            item.as_object_mut()
                .expect("item field requires object")
                .remove("content");
        }
    }
    Ok(input)
}

fn copy_codex_optional_fields(source: &Map<String, Value>, target: &mut Map<String, Value>) {
    for field in ["reasoning", "text"] {
        if let Some(value) = source
            .get(field)
            .and_then(Value::as_object)
            .filter(|value| !value.is_empty())
        {
            target.insert(field.to_string(), Value::Object(value.clone()));
        }
    }
    if let Some(value) = source
        .get("service_tier")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        target.insert("service_tier".to_string(), json!(value));
    }
}

fn insert_prompt_cache_key(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    session_id: Option<&str>,
) {
    let value = source
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| session_id.filter(|value| !value.trim().is_empty()));
    if let Some(value) = value {
        target.insert("prompt_cache_key".to_string(), json!(value));
    }
}

pub(super) fn encode_request(
    protocol: UpstreamProtocol,
    request: Value,
    anthropic_max_tokens: i64,
) -> Result<EncodedRequest, String> {
    if request.get("stream").and_then(Value::as_bool) != Some(true) {
        return Err("首期仅支持 stream=true。".to_string());
    }
    let object = request
        .as_object()
        .ok_or_else(|| "请求 JSON 必须是对象。".to_string())?;
    let model = required_string(object, "model")?;
    let instructions = object
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = object
        .get("input")
        .and_then(Value::as_array)
        .ok_or_else(|| "请求缺少 input 数组。".to_string())?;
    reject_unsupported_input(input)?;
    let (tools, tool_map) = encode_tools(object.get("tools"), protocol)?;
    let tool_choice = encode_tool_choice(object.get("tool_choice"), &tool_map, protocol)?;

    let body = match protocol {
        UpstreamProtocol::OpenAiChatCompletions => openai_chat::encode_body(
            object,
            model,
            instructions,
            input,
            tools,
            &tool_map,
            tool_choice,
        )?,
        UpstreamProtocol::AnthropicMessages => anthropic_messages::encode_body(
            model,
            instructions,
            input,
            tools,
            &tool_map,
            tool_choice,
            anthropic_max_tokens,
        )?,
        UpstreamProtocol::OpenAiResponses => unreachable!(),
    };
    Ok(EncodedRequest {
        body,
        tools: tool_map,
    })
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("请求缺少 {key}。"))
}

pub(super) fn insert_if_some(body: &mut Value, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        body[key] = value;
    }
}

fn reject_unsupported_input(input: &[Value]) -> Result<(), String> {
    for item in input {
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        if !matches!(
            kind,
            "message"
                | "function_call"
                | "function_call_output"
                | "custom_tool_call"
                | "custom_tool_call_output"
                | "reasoning"
        ) {
            return Err(format!("unsupported_feature: 不支持输入项 {kind}。"));
        }
        if kind == "message" {
            for part in item
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
                if !matches!(part_type, "input_text" | "output_text" | "input_image") {
                    return Err(format!("unsupported_feature: 不支持内容块 {part_type}。"));
                }
            }
        }
    }
    Ok(())
}

fn encode_tools(
    tools: Option<&Value>,
    protocol: UpstreamProtocol,
) -> Result<(Vec<Value>, HashMap<String, ToolMeta>), String> {
    let tools = tools
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let reserved_names = tools
        .iter()
        .filter(|tool| tool.get("namespace").and_then(Value::as_str).is_none())
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .filter(|name| valid_tool_name(name))
        .collect::<HashSet<_>>();
    let mut encoded = Vec::new();
    let mut map = HashMap::new();
    let mut used_names = HashSet::new();
    for (index, tool) in tools.iter().enumerate() {
        let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
        if !matches!(kind, "function" | "custom") {
            eprintln!("Model gateway filtered unsupported tool: {kind}");
            continue;
        }
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "工具缺少 name。".to_string())?;
        let namespace = tool
            .get("namespace")
            .and_then(Value::as_str)
            .map(str::to_string);
        let alias = if namespace.is_none() && valid_tool_name(name) {
            if !used_names.insert(name.to_string()) {
                return Err(format!("工具名称重复：{name}。"));
            }
            name.to_string()
        } else {
            let mut suffix = index;
            loop {
                let candidate = format!("cortana_tool_{suffix}");
                suffix += 1;
                if !reserved_names.contains(candidate.as_str())
                    && used_names.insert(candidate.clone())
                {
                    break candidate;
                }
            }
        };
        let custom = kind == "custom";
        map.insert(
            alias.clone(),
            ToolMeta {
                original_name: name.to_string(),
                namespace,
                custom,
            },
        );
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let schema = if custom {
            json!({
                "type": "object",
                "properties": { "input": { "type": "string" } },
                "required": ["input"],
                "additionalProperties": false
            })
        } else {
            tool.get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}))
        };
        encoded.push(match protocol {
            UpstreamProtocol::OpenAiChatCompletions => json!({
                "type": "function",
                "function": { "name": alias, "description": description, "parameters": schema }
            }),
            UpstreamProtocol::AnthropicMessages => json!({
                "name": alias, "description": description, "input_schema": schema
            }),
            UpstreamProtocol::OpenAiResponses => unreachable!(),
        });
    }
    Ok((encoded, map))
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn alias_for_tool<'a>(
    name: &str,
    map: &'a HashMap<String, ToolMeta>,
) -> Option<&'a str> {
    map.iter()
        .find(|(_, meta)| meta.original_name == name)
        .map(|(alias, _)| alias.as_str())
}

fn encode_tool_choice(
    choice: Option<&Value>,
    tools: &HashMap<String, ToolMeta>,
    protocol: UpstreamProtocol,
) -> Result<Option<Value>, String> {
    let Some(choice) = choice else {
        return Ok(None);
    };
    if let Some(choice) = choice.as_str() {
        return Ok(Some(match protocol {
            UpstreamProtocol::OpenAiChatCompletions => json!(choice),
            UpstreamProtocol::AnthropicMessages => match choice {
                "auto" => json!({"type":"auto"}),
                "required" => json!({"type":"any"}),
                "none" => return Ok(None),
                _ => return Err("tool_choice 无效。".to_string()),
            },
            UpstreamProtocol::OpenAiResponses => unreachable!(),
        }));
    }
    let name = choice
        .get("name")
        .or_else(|| choice.get("function").and_then(|value| value.get("name")))
        .and_then(Value::as_str)
        .ok_or_else(|| "tool_choice 无效。".to_string())?;
    let alias = alias_for_tool(name, tools)
        .ok_or_else(|| "tool_choice 指向了不支持或不存在的工具。".to_string())?;
    Ok(Some(match protocol {
        UpstreamProtocol::OpenAiChatCompletions => {
            json!({"type":"function","function":{"name":alias}})
        }
        UpstreamProtocol::AnthropicMessages => json!({"type":"tool","name":alias}),
        UpstreamProtocol::OpenAiResponses => unreachable!(),
    }))
}

pub(super) fn tool_output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

pub(super) fn reasoning_text(item: &Value) -> String {
    item.get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

const SIGNATURE_PREFIX: &str = "cortana:v1:";

fn encode_signature(signature: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    format!(
        "{SIGNATURE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(json!({"signature":signature}).to_string())
    )
}

pub(super) fn decode_signature(value: Option<&str>) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let encoded = value?.strip_prefix(SIGNATURE_PREFIX)?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    serde_json::from_slice::<Value>(&decoded)
        .ok()?
        .get("signature")?
        .as_str()
        .map(str::to_string)
}

pub(super) struct ResponseStream<R: Read> {
    upstream: BufReader<R>,
    protocol: UpstreamProtocol,
    tools: HashMap<String, ToolMeta>,
    pending: VecDeque<u8>,
    state: StreamState,
    ended: bool,
}

#[derive(Default)]
struct StreamState {
    response_id: String,
    sequence: u64,
    next_output: usize,
    text: HashMap<usize, Block>,
    tools: HashMap<usize, ToolBlock>,
    reasoning: HashMap<usize, ReasoningBlock>,
    usage: Value,
    stop_reason: Option<String>,
    output: BTreeMap<usize, Value>,
    buffered_bytes: usize,
}

struct Block {
    output_index: usize,
    item_id: String,
    text: String,
}

struct ToolBlock {
    output_index: usize,
    item_id: String,
    call_id: String,
    alias: String,
    arguments: String,
}

struct ReasoningBlock {
    output_index: usize,
    item_id: String,
    text: String,
    signature: String,
}

impl<R: Read> ResponseStream<R> {
    pub(super) fn new(upstream: R, protocol: UpstreamProtocol, encoded: EncodedRequest) -> Self {
        let response_id = format!("resp_{}", Uuid::new_v4().simple());
        let mut stream = Self {
            upstream: BufReader::new(upstream),
            protocol,
            tools: encoded.tools,
            pending: VecDeque::new(),
            state: StreamState {
                response_id,
                usage: json!({}),
                ..Default::default()
            },
            ended: false,
        };
        stream.emit(
            "response.created",
            json!({"response":stream.response("in_progress")}),
        );
        stream
    }

    fn response(&self, status: &str) -> Value {
        let output = self.state.output.values().cloned().collect::<Vec<_>>();
        json!({
            "id":self.state.response_id,"object":"response","created_at":chrono::Utc::now().timestamp(),
            "status":status,"output":output,"usage":self.state.usage
        })
    }

    fn emit(&mut self, event: &str, mut value: Value) {
        value["type"] = json!(event);
        value["sequence_number"] = json!(self.state.sequence);
        self.state.sequence += 1;
        let bytes = format!("event: {event}\ndata: {value}\n\n").into_bytes();
        self.pending.extend(bytes);
    }

    fn pump(&mut self) -> io::Result<()> {
        let mut line = Vec::new();
        loop {
            line.clear();
            let count =
                match read_line_bounded(&mut self.upstream, &mut line, MAX_UPSTREAM_SSE_LINE_BYTES)
                {
                    Ok(count) => count,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        self.fail("upstream_event_too_large", "上游 SSE 事件过大。");
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
            if count == 0 {
                if !self.ended {
                    self.fail("upstream_stream_interrupted", "上游流意外中断。");
                }
                return Ok(());
            }
            let Ok(line) = std::str::from_utf8(&line) else {
                self.fail("invalid_upstream_event", "上游 SSE 不是有效的 UTF-8。");
                return Ok(());
            };
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                if !self.ended {
                    self.finish();
                }
                return Ok(());
            }
            let value: Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(_) => {
                    self.fail("invalid_upstream_event", "上游返回了无效的 SSE JSON。");
                    return Ok(());
                }
            };
            match self.protocol {
                UpstreamProtocol::OpenAiChatCompletions => openai_chat::handle_event(self, &value),
                UpstreamProtocol::AnthropicMessages => {
                    anthropic_messages::handle_event(self, &value)
                }
                UpstreamProtocol::OpenAiResponses => unreachable!(),
            }
            if !self.pending.is_empty() {
                return Ok(());
            }
        }
    }

    pub(super) fn start_text(&mut self, index: usize) {
        if self.state.text.contains_key(&index) {
            return;
        }
        let output_index = self.state.next_output;
        self.state.next_output += 1;
        let item_id = format!("msg_{}", Uuid::new_v4().simple());
        self.emit("response.output_item.added", json!({"output_index":output_index,"item":{"id":item_id,"type":"message","status":"in_progress","role":"assistant","content":[]}}));
        self.emit("response.content_part.added", json!({"item_id":item_id,"output_index":output_index,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
        self.state.text.insert(
            index,
            Block {
                output_index,
                item_id,
                text: String::new(),
            },
        );
    }

    pub(super) fn text_delta(&mut self, index: usize, delta: &str) {
        if !self.reserve_output_bytes(delta.len()) {
            return;
        }
        self.start_text(index);
        let (item_id, output_index) = {
            let block = self.state.text.get_mut(&index).unwrap();
            block.text.push_str(delta);
            (block.item_id.clone(), block.output_index)
        };
        self.emit(
            "response.output_text.delta",
            json!({"item_id":item_id,"output_index":output_index,"content_index":0,"delta":delta}),
        );
    }

    pub(super) fn end_text(&mut self, index: usize) {
        let Some(block) = self.state.text.remove(&index) else {
            return;
        };
        self.emit("response.output_text.done", json!({"item_id":block.item_id,"output_index":block.output_index,"content_index":0,"text":block.text}));
        self.emit("response.content_part.done", json!({"item_id":block.item_id,"output_index":block.output_index,"content_index":0,"part":{"type":"output_text","text":block.text,"annotations":[]}}));
        let item = json!({"id":block.item_id,"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":block.text,"annotations":[]}]});
        self.emit(
            "response.output_item.done",
            json!({"output_index":block.output_index,"item":item}),
        );
        self.state.output.insert(block.output_index, item);
    }

    pub(super) fn start_tool(&mut self, index: usize, call_id: &str, alias: &str) {
        if self.state.tools.contains_key(&index) {
            return;
        }
        let output_index = self.state.next_output;
        self.state.next_output += 1;
        let item_id = format!("fc_{}", Uuid::new_v4().simple());
        let meta = self.tools.get(alias);
        let kind = if meta.is_some_and(|meta| meta.custom) {
            "custom_tool_call"
        } else {
            "function_call"
        };
        let mut item = json!({"id":item_id,"type":kind,"status":"in_progress","call_id":call_id});
        item["name"] = json!(meta
            .map(|meta| meta.original_name.as_str())
            .unwrap_or(alias));
        if let Some(namespace) = meta.and_then(|meta| meta.namespace.as_deref()) {
            item["namespace"] = json!(namespace);
        }
        if kind == "function_call" {
            item["arguments"] = json!("");
        } else {
            item["input"] = json!("");
        }
        self.emit(
            "response.output_item.added",
            json!({"output_index":output_index,"item":item}),
        );
        self.state.tools.insert(
            index,
            ToolBlock {
                output_index,
                item_id,
                call_id: call_id.to_string(),
                alias: alias.to_string(),
                arguments: String::new(),
            },
        );
    }

    pub(super) fn tool_delta(&mut self, index: usize, delta: &str) {
        if !self.state.tools.contains_key(&index) || !self.reserve_output_bytes(delta.len()) {
            return;
        }
        let Some(block) = self.state.tools.get_mut(&index) else {
            return;
        };
        block.arguments.push_str(delta);
        if !self.tools.get(&block.alias).is_some_and(|meta| meta.custom) {
            let item_id = block.item_id.clone();
            let output_index = block.output_index;
            self.emit(
                "response.function_call_arguments.delta",
                json!({"item_id":item_id,"output_index":output_index,"delta":delta}),
            );
        }
    }

    pub(super) fn end_tool(&mut self, index: usize) {
        let Some(block) = self.state.tools.remove(&index) else {
            return;
        };
        let meta = self.tools.get(&block.alias).cloned().unwrap_or(ToolMeta {
            original_name: block.alias.clone(),
            namespace: None,
            custom: false,
        });
        let valid = serde_json::from_str::<Value>(&block.arguments);
        if valid.is_err() {
            self.fail(
                "invalid_tool_arguments",
                "上游返回的工具参数不是有效 JSON。",
            );
            return;
        }
        let mut item = json!({"id":block.item_id,"status":"completed","call_id":block.call_id,"name":meta.original_name});
        if let Some(namespace) = meta.namespace {
            item["namespace"] = json!(namespace);
        }
        if meta.custom {
            let input = valid
                .ok()
                .and_then(|value| {
                    value
                        .get("input")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            item["type"] = json!("custom_tool_call");
            item["input"] = json!(input);
            self.emit(
                "response.custom_tool_call_input.delta",
                json!({"item_id":block.item_id,"output_index":block.output_index,"delta":input}),
            );
            self.emit(
                "response.custom_tool_call_input.done",
                json!({"item_id":block.item_id,"output_index":block.output_index,"input":input}),
            );
        } else {
            item["type"] = json!("function_call");
            item["arguments"] = json!(block.arguments);
            self.emit("response.function_call_arguments.done", json!({"item_id":block.item_id,"output_index":block.output_index,"arguments":block.arguments}));
        }
        self.emit(
            "response.output_item.done",
            json!({"output_index":block.output_index,"item":item}),
        );
        self.state.output.insert(block.output_index, item);
    }

    pub(super) fn reasoning_delta(&mut self, index: usize, delta: &str) {
        if !self.reserve_output_bytes(delta.len()) {
            return;
        }
        if !self.state.reasoning.contains_key(&index) {
            let output_index = self.state.next_output;
            self.state.next_output += 1;
            let item_id = format!("rs_{}", Uuid::new_v4().simple());
            self.emit("response.output_item.added", json!({"output_index":output_index,"item":{"id":item_id,"type":"reasoning","status":"in_progress","summary":[]}}));
            self.state.reasoning.insert(
                index,
                ReasoningBlock {
                    output_index,
                    item_id,
                    text: String::new(),
                    signature: String::new(),
                },
            );
        }
        let (item_id, output_index) = {
            let block = self.state.reasoning.get_mut(&index).unwrap();
            block.text.push_str(delta);
            (block.item_id.clone(), block.output_index)
        };
        self.emit(
            "response.reasoning_summary_text.delta",
            json!({"item_id":item_id,"output_index":output_index,"summary_index":0,"delta":delta}),
        );
    }

    pub(super) fn end_reasoning(&mut self, index: usize) {
        let Some(block) = self.state.reasoning.remove(&index) else {
            return;
        };
        let encrypted = (!block.signature.is_empty()).then(|| encode_signature(&block.signature));
        self.emit("response.reasoning_summary_text.done", json!({"item_id":block.item_id,"output_index":block.output_index,"summary_index":0,"text":block.text}));
        let item = json!({"id":block.item_id,"type":"reasoning","status":"completed","summary":[{"type":"summary_text","text":block.text}],"encrypted_content":encrypted});
        self.emit(
            "response.output_item.done",
            json!({"output_index":block.output_index,"item":item}),
        );
        self.state.output.insert(block.output_index, item);
    }

    pub(super) fn append_reasoning_signature(&mut self, index: usize, signature: &str) {
        if !self.state.reasoning.contains_key(&index) || !self.reserve_output_bytes(signature.len())
        {
            return;
        }
        if let Some(block) = self.state.reasoning.get_mut(&index) {
            block.signature.push_str(signature);
        }
    }

    fn reserve_output_bytes(&mut self, additional: usize) -> bool {
        let Some(total) = self.state.buffered_bytes.checked_add(additional) else {
            self.fail("upstream_output_too_large", "上游输出过大。");
            return false;
        };
        if total > MAX_CONVERTED_OUTPUT_BYTES {
            self.fail("upstream_output_too_large", "上游输出过大。");
            return false;
        }
        self.state.buffered_bytes = total;
        true
    }

    pub(super) fn set_usage(&mut self, input: u64, output: u64, total: u64) {
        self.state.usage =
            json!({"input_tokens":input,"output_tokens":output,"total_tokens":total});
    }

    pub(super) fn merge_usage(&mut self, input: Option<u64>, output: Option<u64>) {
        let input = input.unwrap_or_else(|| {
            self.state
                .usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        });
        let output = output.unwrap_or_else(|| {
            self.state
                .usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        });
        self.set_usage(input, output, input + output);
    }

    pub(super) fn set_stop_reason(&mut self, reason: Option<&str>) {
        self.state.stop_reason = reason.map(str::to_string);
    }

    pub(super) fn finish(&mut self) {
        let text = self.state.text.keys().copied().collect::<Vec<_>>();
        for index in text {
            self.end_text(index);
        }
        let reasoning = self.state.reasoning.keys().copied().collect::<Vec<_>>();
        for index in reasoning {
            self.end_reasoning(index);
        }
        let tools = self.state.tools.keys().copied().collect::<Vec<_>>();
        for index in tools {
            self.end_tool(index);
        }
        if self.ended {
            return;
        }
        self.ended = true;
        let reason = self.state.stop_reason.as_deref().unwrap_or("stop");
        if matches!(reason, "length" | "max_tokens") {
            let mut response = self.response("incomplete");
            response["incomplete_details"] = json!({"reason":"max_output_tokens"});
            self.emit("response.incomplete", json!({"response":response}));
        } else {
            self.emit(
                "response.completed",
                json!({"response":self.response("completed")}),
            );
        }
    }

    pub(super) fn fail(&mut self, code: &str, message: &str) {
        if self.ended {
            return;
        }
        self.ended = true;
        let mut response = self.response("failed");
        response["error"] = json!({"code":code,"message":message});
        self.emit("response.failed", json!({"response":response}));
    }
}

fn read_line_bounded<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    limit: usize,
) -> io::Result<usize> {
    let mut total = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(total);
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len() + take > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "upstream SSE line too large",
            ));
        }
        line.extend_from_slice(&available[..take]);
        let ended = available[take - 1] == b'\n';
        reader.consume(take);
        total += take;
        if ended {
            return Ok(total);
        }
    }
}

impl<R: Read> Read for ResponseStream<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while self.pending.is_empty() && !self.ended {
            self.pump()?;
        }
        let count = buffer.len().min(self.pending.len());
        for slot in &mut buffer[..count] {
            *slot = self.pending.pop_front().unwrap();
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn event(output: &str, event_type: &str) -> Value {
        output
            .split("\n\n")
            .find_map(|chunk| {
                let data = chunk.lines().find_map(|line| line.strip_prefix("data: "))?;
                let value: Value = serde_json::from_str(data).ok()?;
                (value["type"] == event_type).then_some(value)
            })
            .unwrap()
    }

    #[test]
    fn oauth_request_keeps_only_codex_fields() {
        let request = prepare_codex_oauth_request(
            json!({
                "model":"gpt-test",
                "input":[{"type":"reasoning","content":null,"summary":[]}],
                "tools":[],
                "reasoning":{"effort":"high"},
                "include":[],
                "service_tier":"priority",
                "previous_response_id":"discard",
                "client_metadata":{"existing":true,"x-codex-installation-id":"body-install"}
            }),
            Some("session-1"),
            Some("install-1"),
        )
        .unwrap();
        assert_eq!(request["tool_choice"], "auto");
        assert_eq!(request["parallel_tool_calls"], true);
        assert_eq!(request["prompt_cache_key"], "session-1");
        assert_eq!(request["client_metadata"]["existing"], true);
        assert_eq!(
            request["client_metadata"]["x-codex-installation-id"],
            "body-install"
        );
        assert!(request["input"][0].get("content").is_none());
        assert_eq!(request["include"][0], "reasoning.encrypted_content");
        assert!(request.get("previous_response_id").is_none());
    }

    #[test]
    fn compact_request_drops_responses_only_fields() {
        let request = prepare_codex_compact_request(
            json!({
                "model":"gpt-test","input":[],"tools":[],
                "reasoning":{"effort":"medium"},"text":{"verbosity":"low"},
                "tool_choice":"required","store":true,"stream":true,
                "include":["reasoning.encrypted_content"],
                "client_metadata":{"ignored":true}
            }),
            Some("session-1"),
        )
        .unwrap();
        assert_eq!(request["prompt_cache_key"], "session-1");
        assert_eq!(request["reasoning"]["effort"], "medium");
        for field in [
            "tool_choice",
            "store",
            "stream",
            "include",
            "client_metadata",
        ] {
            assert!(request.get(field).is_none(), "unexpected field: {field}");
        }
    }

    #[test]
    fn codex_request_rejects_invalid_required_fields() {
        assert!(prepare_codex_compact_request(json!({"input":[]}), None).is_err());
        assert!(prepare_codex_oauth_request(
            json!({"model":"gpt-test","input":[],"tools":{}}),
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn chat_request_preserves_images_and_tools() {
        let encoded = encode_request(UpstreamProtocol::OpenAiChatCompletions, json!({
            "model":"gpt-test","stream":true,"instructions":"be brief",
            "input":[{"type":"message","role":"user","content":[
                {"type":"input_text","text":"hi"},{"type":"input_image","image_url":"data:image/png;base64,AA=="}
            ]}],
            "tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}]
        }), 100).unwrap();
        assert_eq!(encoded.body["messages"][0]["role"], "system");
        assert_eq!(
            encoded.body["messages"][1]["content"][1]["type"],
            "image_url"
        );
        assert_eq!(encoded.body["tools"][0]["function"]["name"], "shell");
    }

    #[test]
    fn generated_tool_aliases_do_not_collide_with_real_names() {
        let encoded = encode_request(
            UpstreamProtocol::OpenAiChatCompletions,
            json!({
                "model":"gpt-test","stream":true,"input":[],
                "tools":[
                    {"type":"function","name":"cortana_tool_1"},
                    {"type":"function","name":"shell","namespace":"mcp"}
                ]
            }),
            100,
        )
        .unwrap();
        assert_eq!(
            encoded.body["tools"][0]["function"]["name"],
            "cortana_tool_1"
        );
        assert_eq!(
            encoded.body["tools"][1]["function"]["name"],
            "cortana_tool_2"
        );
    }

    #[test]
    fn rejects_oversized_upstream_sse_lines() {
        let mut reader = BufReader::new(Cursor::new(vec![b'x'; MAX_UPSTREAM_SSE_LINE_BYTES + 1]));
        let mut line = Vec::new();
        let error =
            read_line_bounded(&mut reader, &mut line, MAX_UPSTREAM_SSE_LINE_BYTES).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(line.len() <= MAX_UPSTREAM_SSE_LINE_BYTES);
    }

    #[test]
    fn rejects_unbounded_converted_output() {
        let encoded = encode_request(
            UpstreamProtocol::OpenAiChatCompletions,
            json!({"model":"x","stream":true,"input":[]}),
            10,
        )
        .unwrap();
        let mut stream = ResponseStream::new(
            Cursor::new(Vec::<u8>::new()),
            UpstreamProtocol::OpenAiChatCompletions,
            encoded,
        );
        stream.state.buffered_bytes = MAX_CONVERTED_OUTPUT_BYTES;
        stream.text_delta(0, "x");
        assert!(stream.ended);
        assert!(stream
            .pending
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .windows(b"upstream_output_too_large".len())
            .any(|window| window == b"upstream_output_too_large"));
    }

    #[test]
    fn anthropic_stream_is_not_buffered() {
        let encoded = encode_request(
            UpstreamProtocol::AnthropicMessages,
            json!({"model":"x","stream":true,"input":[]}),
            10,
        )
        .unwrap();
        let upstream = b"data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        let mut stream = ResponseStream::new(
            Cursor::new(upstream),
            UpstreamProtocol::AnthropicMessages,
            encoded,
        );
        let mut output = String::new();
        stream.read_to_string(&mut output).unwrap();
        assert!(output.contains("response.output_text.delta"));
        assert!(output.contains("response.completed"));
    }

    #[test]
    fn chat_stream_preserves_parallel_tool_calls_and_completed_output() {
        let encoded = encode_request(
            UpstreamProtocol::OpenAiChatCompletions,
            json!({
                "model":"x","stream":true,"input":[],
                "tools":[
                    {"type":"function","name":"first","parameters":{"type":"object"}},
                    {"type":"custom","name":"apply_patch"}
                ]
            }),
            10,
        )
        .unwrap();
        let upstream = b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"first\",\"arguments\":\"{\\\"value\\\":\"}},{\"index\":1,\"id\":\"call-2\",\"function\":{\"name\":\"apply_patch\",\"arguments\":\"{\\\"input\\\":\\\"diff\\\"}\"}}]}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let mut stream = ResponseStream::new(
            Cursor::new(upstream),
            UpstreamProtocol::OpenAiChatCompletions,
            encoded,
        );
        let mut output = String::new();
        stream.read_to_string(&mut output).unwrap();
        let completed = event(&output, "response.completed");
        assert_eq!(completed["response"]["output"].as_array().unwrap().len(), 2);
        assert!(output.contains("response.function_call_arguments.delta"));
        assert!(output.contains("response.custom_tool_call_input.done"));
    }

    #[test]
    fn stream_errors_and_length_limits_are_terminal_responses() {
        let request = || {
            encode_request(
                UpstreamProtocol::OpenAiChatCompletions,
                json!({"model":"x","stream":true,"input":[]}),
                10,
            )
            .unwrap()
        };
        let mut failed = ResponseStream::new(
            Cursor::new(b"data: {\"error\":{\"code\":\"bad\",\"message\":\"nope\"}}\n\n"),
            UpstreamProtocol::OpenAiChatCompletions,
            request(),
        );
        let mut output = String::new();
        failed.read_to_string(&mut output).unwrap();
        assert_eq!(
            event(&output, "response.failed")["response"]["error"]["code"],
            "bad"
        );

        let mut incomplete = ResponseStream::new(
            Cursor::new(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n"),
            UpstreamProtocol::OpenAiChatCompletions,
            request(),
        );
        output.clear();
        incomplete.read_to_string(&mut output).unwrap();
        assert_eq!(
            event(&output, "response.incomplete")["response"]["incomplete_details"]["reason"],
            "max_output_tokens"
        );
    }
}
