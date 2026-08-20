import type { IEventProcessor } from '../event-processor'
import type { ScriptErrorEvent } from '../../../types'
import { useGameStore } from '../../../stores/modules/game'
import { useUIStore } from '../../../stores/modules/ui/ui'

export default class ErrorProcessor implements IEventProcessor {
  canHandle(eventType: string): boolean {
    return eventType === 'error'
  }

  async processEvent(event: ScriptErrorEvent): Promise<void> {
    const gameStore = useGameStore()
    const uiStore = useUIStore()

    console.log('处理错误事件:', event)

    // 使用 error_code 查询对应的角色专属提示
    uiStore.showError({
      errorCode: event.error_code || 'default_error',
    })

    // 重置游戏状态。剧本模式下不拉回 input：LLM 失败后 ai_dialogue 会持续重试，
    // 后面还有剧本事件按序覆盖状态；提前 input 会让读档/重试场景误现输入框。
    // 自由对话模式保持原行为（回到可输入状态）。
    if (!gameStore.runningScript) {
      gameStore.currentStatus = 'input'
    }
    gameStore.currentLine = ''
    console.log('游戏状态已重置为: input (由错误处理器触发)')
  }
}
