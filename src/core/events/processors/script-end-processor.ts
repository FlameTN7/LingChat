import type { IEventProcessor } from '../event-processor'
import type { ScriptEndEvent } from '../../../types'
import { useGameStore } from '../../../stores/modules/game'
import { useUIStore } from '../../../stores/modules/ui/ui'
import { WebSocketMessageTypes } from '../../../types'
import { useAdventureStore } from '@/stores/modules/adventure'

export default class ScriptEndProcessor implements IEventProcessor {
  canHandle(eventType: string): boolean {
    return eventType === WebSocketMessageTypes.SCRIPT_END
  }

  async processEvent(event: ScriptEndEvent): Promise<void> {
    // completed === false 表示剧本是因为出错被中止的。仍然要退出剧情模式把
    // UI 放出来，但不能把羁绊记为完成。旧版本没有这个字段，按完成处理。
    const completed = event.completed !== false

    const adventureStore = useAdventureStore()
    if (completed && adventureStore.inProgressAdventures.length > 0) {
      // 按道理来讲，应该只有一个进行的剧本哈，但是为了保险起见，还是遍历一下
      for (const adventure of adventureStore.inProgressAdventures) {
        adventureStore.markAdventureCompleted(adventure.adventure_folder)
      }
    }

    const gameStore = useGameStore()
    // 标记「剧本正常完成」：ScriptCompleteDisplay 据此显示 Story Clear。
    // 失败退出（completed=false）或其他退出路径不置位，避免误显示「本次剧本已完成」。
    gameStore.scriptEndCompleted = completed
    gameStore.exitStoryMode()
    const uiStore = useUIStore()
    uiStore.showPlayerHintLine = ''
  }
}
