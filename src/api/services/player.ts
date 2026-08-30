import { invoke } from '@tauri-apps/api/core'
import type { PlayerProfile } from './game-info'

/**
 * 读取全局玩家档案（文件驱动）。
 */
export const getPlayerProfile = async (): Promise<PlayerProfile> => {
  return await invoke<PlayerProfile>('get_player_profile')
}

/**
 * 保存全局玩家档案。
 */
export const setPlayerProfile = async (
  user_name: string,
  user_subtitle?: string,
  user_prompt?: string,
  info?: string,
  system_prompt_example?: string,
): Promise<{ success: boolean }> => {
  return await invoke<{ success: boolean }>('set_player_profile', {
    userName: user_name,
    userSubtitle: user_subtitle,
    userPrompt: user_prompt,
    info,
    systemPromptExample: system_prompt_example,
  })
}

/**
 * 保存玩家头像（base64 图片数据写入 game_data/player/头像.<ext>）。
 */
export const savePlayerAvatar = async (
  imageBase64: string,
  ext?: string,
): Promise<{ success: boolean; filename: string; avatar_path: string }> => {
  return await invoke<{ success: boolean; filename: string; avatar_path: string }>(
    'save_player_avatar',
    { imageBase64, ext },
  )
}
