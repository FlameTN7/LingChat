import type { IEventProcessor } from '../event-processor'
import type { ScriptRetryEvent } from '../../../types'
import { useGameStore } from '@/stores/modules/game'
import { useUIStore } from '@/stores/modules/ui/ui'

/**
 * script:llm-retry —— AI 对话的 LLM 调用自动重试耗尽后，等待玩家点击「继续」重试。
 *
 * 进入「等待继续」状态（responding + 提示文本）：玩家点击继续走 GameDialog 的
 * 继续按钮 → eventQueue.continue() → script_event_continue 回执给引擎，
 * 引擎据此重新调用 LLM。不跳过对话、不退出剧本（剧本连续性优先）。
 */
export default class RetryProcessor implements IEventProcessor {
  canHandle(eventType: string): boolean {
    return eventType === 'retry'
  }

  async processEvent(event: ScriptRetryEvent): Promise<void> {
    const gameStore = useGameStore()
    const uiStore = useUIStore()

    gameStore.currentStatus = 'responding'
    uiStore.showCharacterLine = event.message || 'AI 响应失败，点击继续重试'
    uiStore.showCharacterTitle = ''
    uiStore.showCharacterSubtitle = ''
    uiStore.showCharacterMotionText = ''
  }
}
