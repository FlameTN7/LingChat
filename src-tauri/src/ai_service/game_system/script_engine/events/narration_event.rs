//! Narration event — displays narrator text and adds an ASSISTANT line.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::ai_service::game_system::script_engine::events::{
    parse_duration, register_event, wait_for_frontend_continue, ScriptContext, ScriptEvent,
};
use crate::ai_service::game_system::script_engine::responses::{
    event_names::SCRIPT_NARRATION, NarrationPayload,
};
use crate::ai_service::message_system::events::emit;
use crate::ai_service::types::{LineAttributeExt, LineBase};
use crate::db::entities::line::LineAttribute;
use crate::utils::prompt::PromptRole;

pub struct NarrationEvent {
    text: String,
    display_name: Option<String>,
    duration: Option<f64>,
}

impl NarrationEvent {
    fn from_event_data(data: &Value) -> Self {
        Self {
            text: data
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            display_name: data
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            duration: parse_duration(data),
        }
    }
}

#[async_trait]
impl ScriptEvent for NarrationEvent {
    async fn execute(&mut self, ctx: &mut ScriptContext<'_>) -> Result<Option<String>> {
        // Split text by newlines and emit each line separately
        let lines: Vec<&str> = self
            .text
            .split('\n')
            .filter(|line| !line.is_empty())
            .collect();

        // 逐行 emit + 逐行等待前端「继续」：
        // 1) 让引擎与玩家阅读同步（预跑否则会把后续剧情整段写进 line_list，存档捕获超前内容）；
        // 2) 逐行等待使「点击继续的次数」与「等待次数」严格 1:1，多行旁白不会产生多余回执
        //    提前推进下一个事件。
        for line in lines {
            let payload = NarrationPayload {
                text: line.to_string(),
                display_name: self.display_name.clone(),
                duration: self.duration,
            };
            let _ = emit(ctx.app, SCRIPT_NARRATION, &payload);
            // 带 duration（YAML 明确自动推进）时不阻塞，与前端 shouldWaitForUser 语义一致；
            // 缺省则逐行等待玩家「继续」，让台词写入与阅读同步（见 execute 头注释）。
            if self.duration.is_none() {
                wait_for_frontend_continue(&ctx.channels).await;
            }
        }

        // 玩家读完所有行后才写入台词（line_list 与玩家阅读对齐，存档不捕获超前内容）
        let line = LineBase {
            content: PromptRole::Narrator.build_prompt(&self.text.clone()),
            attribute: LineAttributeExt(LineAttribute::User),
            display_name: self.display_name.clone().or_else(|| Some("旁白".into())),
            sender_role_id: Some(0),
            ..Default::default()
        };
        ctx.game_status.lock().await.add_line(ctx.db, line).await?;

        Ok(None)
    }

    fn event_type() -> &'static str {
        "narration"
    }

    fn duration(&self) -> Option<f64> {
        self.duration
    }
}

pub fn register() {
    register_event(NarrationEvent::event_type(), |data| {
        Box::new(NarrationEvent::from_event_data(&data))
    });
}
