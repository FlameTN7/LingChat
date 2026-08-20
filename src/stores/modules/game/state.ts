import type { SceneInfo } from '@/api/services/scene' // 导入场景类型
import type { ScriptChoiceItem } from '@/types/script'

export interface GameMessage {
  type: 'message' | 'reply'
  displayName: string
  content: string
  emotion?: string
  audioFile?: string
  isFinal?: boolean
  motionText?: string
  originalTag?: string
  timestamp?: number
  /** 玩家消息序号（1-indexed），用于回溯定位 */
  userMessageSeq?: number
  /** 该轮生成的思考链（仅每轮最后一条回复消息有值） */
  thinking?: string
  /** 该台词的第二语言（日语）译文，日文界面下显示 */
  ttsText?: string
  /** 台词关联的角色 ID（null = 无角色，如工具调用回填行；生成语音计数时跳过） */
  senderRoleId?: number | null
}

export interface FreeDialogueInfo {
  isFreeDialogue: boolean
  maxRounds: number
  endLine: string
  currentRound: number
}

export interface ScriptInfo {
  scriptName: string
  currentChapterName: string
  choices: ScriptChoiceItem[]
  isRunning: boolean
  freeDialogueInfo: FreeDialogueInfo
}

export interface GameRole {
  roleId: number
  roleName: string
  roleSubTitle: string
  thinkMessage: string
  emotion: string
  originalEmotion: string
  scale: number
  offsetY: number
  offsetX: number
  scaleP: number
  offsetXP: number
  offsetYP: number
  bubbleTop: number
  bubbleLeft: number
  show: boolean
  clothes: object
  clothesName: string
  bodyPart: object
  character_folder: string
}

export interface GameState {
  runningScript: ScriptInfo | null

  gameRoles: Record<number, GameRole>
  presentRoleIds: number[]
  mainRoleId: number
  currentInteractRoleId: number | null

  userName: string
  userSubtitle: string

  currentLine: string
  currentStatus: 'input' | 'thinking' | 'responding' | 'presenting'
  /** 当前思考链累计字数（用于实时显示“已深度思考 N 字”） */
  thinkingLength: number
  dialogHistory: GameMessage[]
  currentScene: SceneInfo | null // 当前加载的场景
  command: string | null

  /** 最近一次 script:progress 广播的事件序号（其后的内容事件即对应此序号，瞬态不持久化） */
  pendingScriptSeq: number
  /** 最近一次 script:progress 广播的章节 key（瞬态不持久化） */
  pendingChapter: string
  /** 玩家最近「已展示」的剧本事件序号：读档时上报给后端，作为玩家阅读位置（瞬态不持久化） */
  displayedSeq: number
  /** 玩家最近「已展示」的章节 key（瞬态不持久化） */
  displayedChapter: string
  /** 剧本是否正常完成（script:end 的 completed=true）：供「剧本已完成」提示判断。
   *  读档续跑失败/手动退出等「非完成退出」不置位，避免误显示 Story Clear（瞬态不持久化） */
  scriptEndCompleted: boolean

  initialized: boolean
  latestScreenshot: string | null
  /** 正在进行的截图 Promise，供 save handler 等待 */
  screenshotPending: Promise<string | null> | null
}

export const state: GameState = {
  runningScript: null,

  gameRoles: {},
  presentRoleIds: [],
  mainRoleId: -1,
  currentInteractRoleId: -1,

  userName: '',
  userSubtitle: '',

  currentLine: '',
  currentStatus: 'input',
  thinkingLength: 0,
  dialogHistory: [],
  currentScene: null,
  command: null,

  pendingScriptSeq: 0,
  pendingChapter: '',
  displayedSeq: 0,
  displayedChapter: '',
  scriptEndCompleted: false,

  initialized: false,
  latestScreenshot: null,
  screenshotPending: null,
}
