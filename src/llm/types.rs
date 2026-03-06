use rig::completion::Usage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuredLearningResponse {
    #[serde(default)]
    pub response: Vec<KnowledgeResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeResponse {
    #[serde(default)]
    pub knowledge_type_group: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub quiz: Vec<QuizItem>,
    #[serde(default)]
    pub knowledge_type_language: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuizItem {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub options: Vec<QuizOption>,
    #[serde(default)]
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuizOption {
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub is_correct_answer: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl From<Usage> for LlmUsage {
    fn from(value: Usage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            total_tokens: value.total_tokens,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LearningGenerationResult {
    pub response: StructuredLearningResponse,
    pub usage: Option<LlmUsage>,
}
