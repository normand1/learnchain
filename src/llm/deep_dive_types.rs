use std::path::PathBuf;

use rig::completion::Usage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::LlmUsage;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepDiveDocument {
    pub metadata: DeepDiveArtifactMetadata,
    pub markdown: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepDiveArtifactMetadata {
    pub artifact_type: String,
    pub title: String,
    pub generated_at: String,
    pub session_source: String,
    pub session_id: String,
    pub session_timestamp: String,
    pub session_date: String,
    pub project_name: String,
    pub project_cwd: String,
    pub source_file: String,
    pub referenced_url_count: usize,
    pub reviewed_url_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepDiveHistoryEntry {
    pub metadata: DeepDiveArtifactMetadata,
    pub path: PathBuf,
    pub file_modified_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeepDiveResearchPlan {
    #[serde(default)]
    pub inferred_goal: String,
    #[serde(default)]
    pub candidate_accomplishments: Vec<String>,
    #[serde(default)]
    pub candidate_interesting_learnings: Vec<String>,
    #[serde(default)]
    pub teaching_angles: Vec<String>,
    #[serde(default)]
    pub selected_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeepDiveReviewedSource {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub why_it_matters: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuredDeepDiveResponse {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub accomplishments: Vec<String>,
    #[serde(default)]
    pub interesting_learnings: Vec<String>,
    #[serde(default)]
    pub teaching_narrative: Vec<String>,
    #[serde(default)]
    pub reviewed_sources: Vec<DeepDiveReviewedSource>,
}

#[derive(Debug, Clone, Default)]
pub struct DeepDiveGenerationResult {
    pub document: DeepDiveDocument,
    pub response: StructuredDeepDiveResponse,
    pub usage: Option<LlmUsage>,
    pub reviewed_source_failures: Vec<String>,
}

impl From<Usage> for DeepDiveGenerationResult {
    fn from(value: Usage) -> Self {
        Self {
            document: DeepDiveDocument::default(),
            response: StructuredDeepDiveResponse::default(),
            usage: Some(value.into()),
            reviewed_source_failures: Vec::new(),
        }
    }
}
