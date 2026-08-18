import type { ScriptEventType } from '../../types'
import { eventProcessorManager } from './event-processor'
import { useGameStore } from '../../stores/modules/game'
import { invoke } from '@tauri-apps/api/core'

// 玩家「真正读到」的内容事件类型：只有这些才推进玩家阅读位置。
// 背景/音乐/特效等瞬时事件不代表阅读进度（引擎预跑它们时玩家还没看到），
// 记录它们会让读档位置越过玩家实际读到的内容。
const PLAYER_READ_EVENT_TYPES = new Set([
  'narration',
  'player',
  'reply',
  'free_dialogue',
  'input',
  'choice',
  'present_pic',
])

export class EventQueue {
  private queue: ScriptEventType[] = []
  private isProcessing = false
  private paused = true
  private currentEvent: ScriptEventType | null = null
  private currentResolve: (() => void) | null = null

  addEvent(event: ScriptEventType) {
    if ((event.type === 'error' || event.type === 'status_reset') && this.currentResolve) {
      this.currentResolve()
      this.currentResolve = null
      this.queue = []
    }
    this.queue.push(event)
    if (!this.isProcessing && !this.paused) {
      this.processQueue()
    }
  }

  private async processQueue() {
    this.isProcessing = true
    try {
      while (this.queue.length > 0) {
        const event = this.queue.shift()
        if (event) {
          // 如果当前事件是thinking类型，且队列后面还有别的事件，则跳过
          if (event.type === 'thinking' && this.queue.length > 0) {
            continue
          }
          this.currentEvent = event
          try {
            await this.processSingleEvent(event)
          } catch (error) {
            console.error('处理事件失败:', error, event)
            this.resetToInputState()
          }
        }
      }
    } finally {
      this.isProcessing = false
      if (this.currentEvent?.isFinal) {
        this.resetToInputState()
      }
    }
  }

  private async processSingleEvent(event: ScriptEventType): Promise<void> {
    // 玩家阅读位置：内容事件展示时，用最近一次 script:progress 的（章节 + 序号）
    // 作为「玩家已读到这」的位置，并上报后端暂存（手动/自动存档时据此写入存档）。
    // 事件队列保序，progress 与其内容事件相邻，故此处读到的 pending 位置正对应当前内容。
    if (PLAYER_READ_EVENT_TYPES.has(event.type)) {
      const gameStore = useGameStore()
      if (gameStore.pendingScriptSeq > 0) {
        gameStore.displayedChapter = gameStore.pendingChapter
        gameStore.displayedSeq = gameStore.pendingScriptSeq
        invoke('update_player_read_position', {
          chapter: gameStore.pendingChapter,
          seq: gameStore.pendingScriptSeq,
        }).catch((e) => console.warn('update_player_read_position 失败:', e))
      }
    }

    // 处理事件并等待完成
    await eventProcessorManager.processEvent(event)

    // 如果事件需要等待用户继续，就等待
    if (this.shouldWaitForUser(event)) {
      await this.waitForUserContinue()
    } else {
      await this.waitForDuration(event.duration)
      console.log('等待' + event.duration + '秒')
    }
  }

  private shouldWaitForUser(event: ScriptEventType): boolean {
    // 明确检查 duration 是否为 null 或 undefined
    if (event.duration === null || event.duration === undefined) {
      return true // 没有设置 duration，等待用户
    }

    // duration 为负数时等待用户
    if (event.duration < 0) {
      return true
    }

    // duration 为 0 或正数时，不等待用户
    return false
  }

  private waitForUserContinue(): Promise<void> {
    return new Promise((resolve) => {
      this.currentResolve = resolve
      // 设置游戏状态为等待用户输入
      const gameStore = useGameStore()
      gameStore.currentStatus = 'responding'
    })
  }

  // 用户继续的方法
  public continue(): boolean {
    let needWait = false // 这个用于标记下个消息是否还没到来，要想继续还需要等待的信号

    if (this.currentResolve) {
      this.currentResolve()
      this.currentResolve = null
    }

    // 假如当前消息不是最后一个，但是队列事件已经没了
    if (!this.currentEvent?.isFinal && this.queue.length === 0) {
      needWait = true
      console.log('后面的消息还没到，请稍等，最后一个消息是:', this.currentEvent)
    }

    return needWait
  }

  clear() {
    this.queue = []
    this.isProcessing = false
    this.paused = true
    this.currentResolve = null
    this.resetToInputState()
  }

  /** 恢复事件队列消费（MainChat 就绪后调用） */
  resume() {
    this.paused = false
    if (this.queue.length > 0 && !this.isProcessing) {
      this.processQueue()
    }
  }

  private resetToInputState() {
    this.currentEvent = null

    const gameStore = useGameStore()
    gameStore.currentStatus = 'input'
    gameStore.currentLine = ''
  }

  getState() {
    return {
      queueLength: this.queue.length,
      isProcessing: this.isProcessing,
      isWaitingForUser: this.currentResolve !== null,
    }
  }

  private waitForDuration(duration: number): Promise<void> {
    return new Promise((resolve) => {
      setTimeout(resolve, duration * 1000)
    })
  }
}

export const eventQueue = new EventQueue()
