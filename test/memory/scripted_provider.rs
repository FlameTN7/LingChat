//! Deterministic provider behavior used by offline validation tooling.
//! The production LlmProvider trait remains the single provider abstraction.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::stream;
use reqwest::Client;

use crate::ai_service::llm::{
    ChunkStream, LlmClient, LlmConfig, LlmModelInfo, LlmProvider, LlmSlot,
};
use crate::ai_service::types::LlmMessage;

#[derive(Clone, Debug, Default)]
pub struct ScriptedProvider {
    pub delay_ms: u64,
    pub fail_section: Option<String>,
    pub empty_section: Option<String>,
    pub panic_section: Option<String>,
    pub calls: Arc<AtomicUsize>,
}

impl ScriptedProvider {
    pub fn response(&self, section: &str) -> Result<String, String> {
        if self.panic_section.as_deref() == Some(section) {
            panic!("scripted provider panic requested for section");
        }
        if self.fail_section.as_deref() == Some(section) {
            return Err(format!("scripted failure for {section}"));
        }
        if self.empty_section.as_deref() == Some(section) {
            return Ok(String::new());
        }
        Ok(format!("[scripted:{section}]"))
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }

    /// Build a real production LlmClient backed by this deterministic provider.
    pub fn slot(self) -> LlmSlot {
        let client = LlmClient::new(
            LlmConfig {
                provider: "scripted".into(),
                model: "scripted".into(),
                api_key: String::new(),
                base_url: String::new(),
                timeout_secs: 30,
                temperature: None,
                top_p: None,
                enable_thinking: false,
                reasoning_effort: None,
                fast_mode: false,
            },
            Client::new(),
            Box::new(self),
        );
        Arc::new(tokio::sync::RwLock::new(Some(Arc::new(client))))
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn list_models(&self, _http: &Client) -> Result<Vec<LlmModelInfo>> {
        Ok(Vec::new())
    }

    async fn complete(&self, _http: &Client, messages: &[LlmMessage]) -> Result<String> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        let prompt = messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or_default();
        let section = if prompt.contains("短期上下文摘要") {
            "short_term"
        } else if prompt.contains("角色经历编年史") {
            "long_term"
        } else if prompt.contains("taの画像") {
            "user_info"
        } else if prompt.contains("待办与契约清单") {
            "promises"
        } else {
            "unknown"
        };
        self.response(section).map_err(|e| anyhow!(e))
    }

    async fn complete_stream(
        &self,
        _http: &Client,
        _messages: &[LlmMessage],
    ) -> Result<ChunkStream> {
        Ok(Box::pin(stream::empty()))
    }
}
