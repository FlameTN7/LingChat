//! Chapter — wraps a chapter YAML config and runs its events sequentially.
//!
//! Replaces Python `Chapter` class.

use anyhow::Result;
use serde_json::Value;

use crate::ai_service::game_system::script_engine::events::ScriptContext;
use crate::ai_service::game_system::script_engine::events_handler::EventsHandler;
use crate::ai_service::game_system::script_engine::responses::{
    event_names::SCRIPT_CHAPTER_CHANGE, ChapterChangePayload,
};
use crate::ai_service::message_system::events::emit;
use crate::ai_service::types::ScriptStatus;

/// A chapter loaded from a chapter YAML file.
pub struct Chapter {
    /// Chapter identifier (the YAML file path relative to the script).
    pub _chapter_id: String,
    /// Display name from the chapter config.
    pub chapter_name: String,
    /// Sequential event processor for this chapter.
    pub events_handler: EventsHandler,
}

impl Chapter {
    /// Construct a `Chapter` from a chapter config dict and script status.
    pub fn new(chapter_id: String, chapter_config: Value, script_status: &ScriptStatus) -> Self {
        let chapter_name = chapter_config
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&chapter_id)
            .to_string();

        let event_list = chapter_config
            .get("events")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // 读档续跑恢复：若当前章节与存档记录章节一致，则从存档事件序号继续执行。
        // 旧存档事件序号为 0（或已越界），分别回退为从章节开头重放 / 钳制到最后一个事件，
        // 保证 `chapter_end` 仍会执行、故事能继续。
        let resume_progress = if script_status.current_chapter_key == chapter_id {
            script_status.current_event_process.max(0) as usize
        } else {
            0
        };

        let mut events_handler = EventsHandler::with_progress(event_list, resume_progress);
        // 记录章节 key，供 script:progress 广播携带（前端记录玩家阅读位置所在章节）
        events_handler.chapter_key = chapter_id.clone();

        Self {
            _chapter_id: chapter_id,
            chapter_name,
            events_handler,
        }
    }

    /// Run all events in this chapter.
    /// Returns the name of the next chapter to load.
    pub async fn run(&mut self, ctx: &mut ScriptContext<'_>) -> Result<String> {
        // Emit chapter_change event to frontend
        let payload = ChapterChangePayload {
            chapter_name: self.chapter_name.clone(),
        };
        let _ = emit(ctx.app, SCRIPT_CHAPTER_CHANGE, &payload);

        tracing::info!(
            "[ScriptEngine] 开始章节: '{}' ({} events, resume from #{})",
            self.chapter_name,
            self.events_handler.event_list.len(),
            self.events_handler.progress
        );

        // Execute events one by one
        while !self.events_handler.is_finished() {
            // 在事件执行前记录「即将执行」的事件序号：保守位置，保证存档点永不超前于已展示的剧情
            // （若在事件执行中存档，续跑会重放该事件而非跳过后续剧情）。
            if let Some(ref mut ss) = ctx.game_status.lock().await.script_status {
                ss.current_event_process = self.events_handler.progress as i32;
            }
            self.events_handler.process_next_event(ctx).await?;
        }

        let result = self.events_handler.get_chapter_result();
        tracing::info!(
            "[ScriptEngine] 章节 '{}' 结束 → 下一章节: '{}'",
            self.chapter_name,
            result
        );

        Ok(result)
    }
}
