import type { IEventProcessor } from '../event-processor'
import type { ScriptThinkingEvent } from '../../../types'
import { useGameStore } from '@/stores/modules/game'

export default class ThinkingProcessor implements IEventProcessor {
  canHandle(eventType: string): boolean {
    return eventType === 'thinking'
  }

  async processEvent(event: ScriptThinkingEvent): Promise<void> {
    const gameStore = useGameStore()

    if (event.isThinking) {
      // AI 开始思考，锁定输入
      gameStore.currentStatus = 'thinking'
      gameStore.thinkingLength = 0
    } else {
      // AI 停止思考。自由对话模式回到可输入状态；剧本模式下不拉回 'input'：
      // 后面还有剧本事件（narration/player/input…）会按序覆盖状态，提前回到
      // input 会让 AI 对话读档/续跑场景误现输入框（玩家名 + 输入框）。
      if (!gameStore.runningScript) {
        gameStore.currentStatus = 'input'
      }
      gameStore.currentLine = ''
      gameStore.thinkingLength = 0
    }
  }
}
