import type { IEventProcessor } from '../event-processor'
import type { ScriptProgressEvent } from '../../../types'
import { useGameStore } from '../../../stores/modules/game'

/**
 * script:progress —— 引擎每实际执行一个事件前广播（章节 key + 事件序号）。
 * 本处理器在事件队列里与其后的内容事件相邻、保序执行：先更新 pending 位置，
 * 随后被展示的内容事件（narration/player/dialogue/…）即可据此记录玩家阅读位置。
 */
export default class ProgressProcessor implements IEventProcessor {
  canHandle(eventType: string): boolean {
    return eventType === 'progress'
  }

  async processEvent(event: ScriptProgressEvent): Promise<void> {
    const gameStore = useGameStore()
    gameStore.pendingChapter = event.chapter
    gameStore.pendingScriptSeq = event.seq
  }
}
