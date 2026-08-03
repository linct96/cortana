use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_ANTHROPIC_MAX_TOKENS: i64 = 16_384;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GatewayStatus {
    pub(crate) enabled: bool,
    pub(crate) available: bool,
    pub(crate) active_profile_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum UpstreamProtocol {
    #[default]
    #[serde(rename = "openaiResponses")]
    OpenAiResponses,
    #[serde(rename = "openaiChatCompletions")]
    OpenAiChatCompletions,
    #[serde(rename = "anthropicMessages")]
    AnthropicMessages,
}

impl UpstreamProtocol {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "openaiResponses",
            Self::OpenAiChatCompletions => "openaiChatCompletions",
            Self::AnthropicMessages => "anthropicMessages",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "openaiResponses" => Ok(Self::OpenAiResponses),
            "openaiChatCompletions" => Ok(Self::OpenAiChatCompletions),
            "anthropicMessages" => Ok(Self::AnthropicMessages),
            _ => Err("上游 API 协议无效。".to_string()),
        }
    }

    pub(crate) fn requires_gateway(self) -> bool {
        self != Self::OpenAiResponses
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum UpstreamAuthMode {
    #[default]
    #[serde(rename = "bearer")]
    Bearer,
    #[serde(rename = "xApiKey")]
    XApiKey,
}

impl UpstreamAuthMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::XApiKey => "xApiKey",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bearer" => Ok(Self::Bearer),
            "xApiKey" => Ok(Self::XApiKey),
            _ => Err("上游认证方式无效。".to_string()),
        }
    }
}
