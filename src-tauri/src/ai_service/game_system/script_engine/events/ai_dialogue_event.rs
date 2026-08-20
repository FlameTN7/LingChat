//! AI 对话事件 —— 设定角色，并通过 MessageGenerator 生成 AI 回复。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use tauri::Manager;

use crate::ai_service::game_system::script_engine::events::{
    parse_duration, register_event, wait_for_frontend_continue, ScriptContext, ScriptEvent,
};
use crate::ai_service::game_system::script_engine::responses::{
    event_names::SCRIPT_LLM_RETRY, LlmRetryPayload,
};
use crate::ai_service::game_system::script_engine::utils::script_function;
use crate::ai_service::message_system::events::emit;
use crate::ai_service::message_system::generator::{
    GeneratorDeps, GeneratorSource, MessageGenerator,
};
use crate::ai_service::types::{LineAttributeExt, LineBase};
use crate::db::entities::line::LineAttribute;
use crate::utils::prompt::PromptRole;
use crate::AppState;

pub struct AIDialogueEvent {
    character: String,
    prompt: Option<String>,
    duration: Option<f64>,
}

impl AIDialogueEvent {
    fn from_event_data(data: &Value) -> Self {
        Self {
            character: data
                .get("character")
                .and_then(|v| v.as_str())
                .unwrap_or("MAIN")
                .to_string(),
            prompt: data
                .get("prompt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            duration: parse_duration(data),
        }
    }
}

#[async_trait]
impl ScriptEvent for AIDialogueEvent {
    async fn execute(&mut self, ctx: &mut ScriptContext<'_>) -> Result<Option<String>> {
        let script_status = ctx
            .game_status
            .lock()
            .await
            .script_status
            .clone()
            .ok_or_else(|| anyhow!("ScriptStatus 未设置"))?;

        let role_id = {
            let mut gs = ctx.game_status.lock().await;
            let role = script_function::get_role(&mut *gs, ctx.db, &script_status, &self.character)
                .await?;
            role.role_id.ok_or_else(|| anyhow!("角色 ID 未设置"))?
        };

        // 设为当前角色
        ctx.game_status.lock().await.current_role_id = Some(role_id);

        tracing::info!("[AIDialogueEvent] 开始执行");

        // 若提供了 prompt，作为临时系统旁白台词注入
        // TODO: 这里的 prompt 是暂时的，应该标记为临时 prompt，并且在代码逻辑中在AI回复后清除这部分提示词。
        if let Some(ref prompt) = self.prompt {
            let sys_line = LineBase {
                content: PromptRole::Plot.build_prompt(prompt),
                attribute: LineAttributeExt(LineAttribute::User),
                display_name: Some("旁白".to_string()),
                ..Default::default()
            };
            ctx.game_status
                .lock()
                .await
                .add_line(ctx.db, sys_line)
                .await?;
        }

        // 委托 MessageGenerator 生成回复
        let state = ctx.app.state::<AppState>();
        let llm = crate::ai_service::llm::slot_snapshot(&state.chat.llm).await;
        let llm = match llm {
            Some(llm) => llm,
            None => {
                // LLM 未配置：AI 对话事件无法生成。按上游要求直接终止剧本，
                // 不再 fallback 到任何占位/默认文本——那会让剧本以错误逻辑继续跑。
                return Err(anyhow!(
                    "尚未配置大模型，无法执行「AI 对话」事件，剧本终止。请先在设置里配置并选择模型。"
                ));
            }
        };

        let deps = GeneratorDeps {
            source: GeneratorSource::ScriptAiDialogue,
            app: ctx.app.clone(),
            db: ctx.db.clone(),
            game_status: ctx.game_status.clone(),
            processor: state.chat.processor.clone(),
            translator: state.chat.translator.clone(),
            llm,
            tool_registry: state.tool_registry.clone(),
            concurrency: 1,
            god_agent: None,
            suppress_thinking: false,
            // 捕获当前试玩代号：中止后游离任务再写会被 add_assistant_line 的守卫丢弃
            generation: ctx.game_status.lock().await.preview_generation,
            is_preview: ctx.is_preview,
        };

        let generator = MessageGenerator::new(deps);

        // LLM 调用失败**绝不踢出玩家**（剧本连续性优先，不跳过对话、不退出剧本）：
        // 1) 先自动重试 3 次（退避约 1s/2s/3s，覆盖瞬时网络抖动）；
        // 2) 仍失败则广播「重试」提示并等待玩家点击「继续」——玩家点继续经
        //    `script_event_continue` 回执到 continue_tx（与旁白共用等待通道，
        //    引擎顺序执行无冲突），此后重置自动重试计数再来一轮；
        // 3) 玩家不想等可退出剧本：stop_script abort 本任务即终止。
        // 重试回溯点正确性：失败时 process_next_event 未返回（进度仍停在当前
        // ai_dialogue）、add_assistant_line 未执行（line_list 无残留），重新生成
        // 的上下文与首次一致。
        let mut attempt: u64 = 0;
        loop {
            match generator.process_message(None).await {
                Ok(_) => break,
                Err(e) => {
                    attempt += 1;
                    tracing::warn!(
                        "[AIDialogueEvent] LLM 生成失败（第 {} 次重试）: {}",
                        attempt,
                        e
                    );
                    if attempt > 3 {
                        // 自动重试耗尽：交还玩家控制
                        let _ = emit(
                            ctx.app,
                            SCRIPT_LLM_RETRY,
                            &LlmRetryPayload {
                                message: format!(
                                    "AI 响应失败（已尝试 {} 次），点击「继续」重试",
                                    attempt
                                ),
                            },
                        );
                        wait_for_frontend_continue(&ctx.channels).await;
                        attempt = 0;
                        continue;
                    }
                    // 自动重试退避（约 1s/2s/3s）
                    tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
                }
            }
        }

        tracing::info!("[AIDialogueEvent] 执行完毕");

        Ok(None)
    }

    fn event_type() -> &'static str {
        "ai_dialogue"
    }

    fn duration(&self) -> Option<f64> {
        self.duration
    }
}

pub fn register() {
    register_event(AIDialogueEvent::event_type(), |data| {
        Box::new(AIDialogueEvent::from_event_data(&data))
    });
}
