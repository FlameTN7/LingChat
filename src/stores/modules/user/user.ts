import { defineStore } from "pinia";
import { getPlayerProfile, savePlayerAvatar, setPlayerProfile } from "@/api/services/player";
import type { PlayerProfile } from "@/api/services/game-info";

export const useUserStore = defineStore("user", {
  state: () => ({
    user_id: "1",
    client_id: "",
    /** 全局玩家档案（解耦玩家与 AI 设定，文件驱动） */
    playerProfile: {
      user_name: "玩家",
      user_subtitle: "",
      user_prompt: "",
      info: "",
      system_prompt_example: "",
      avatar_path: null,
    } as PlayerProfile,
    /** player_profile 是否已加载 */
    profileLoaded: false,
  }),
  getters: {
    /** 玩家名（快捷访问） */
    playerName: (state) => state.playerProfile.user_name,
    /** 玩家副标题 */
    playerSubtitle: (state) => state.playerProfile.user_subtitle,
    /** 玩家系统提示词（设定块） */
    playerPrompt: (state) => state.playerProfile.user_prompt,
    /** 玩家简介 */
    playerInfo: (state) => state.playerProfile.info,
    /** 玩家说话风格示例 */
    playerPromptExample: (state) => state.playerProfile.system_prompt_example,
    /** 玩家头像路径 */
    playerAvatar: (state) => state.playerProfile.avatar_path,
  },
  actions: {
    /** 从后端加载玩家档案 */
    async loadPlayerProfile() {
      try {
        const profile = await getPlayerProfile();
        this.playerProfile = {
          user_name: profile.user_name || "玩家",
          user_subtitle: profile.user_subtitle || "",
          user_prompt: profile.user_prompt || "",
          info: profile.info || "",
          system_prompt_example: profile.system_prompt_example || "",
          avatar_path: profile.avatar_path ?? null,
        };
        this.profileLoaded = true;
      } catch (e) {
        console.warn("加载玩家档案失败:", e);
        this.profileLoaded = false;
      }
    },

    /** 保存玩家档案 */
    async savePlayerProfile(profile: Partial<PlayerProfile>) {
      // 先对当前档案做浅快照：后端保存失败时回滚本地乐观更新，
      // 避免设置弹窗仍停留在“已保存”的表象。
      const snapshot = { ...this.playerProfile };
      this.playerProfile = {
        ...this.playerProfile,
        ...profile,
      };
      try {
        const result = await setPlayerProfile(
          this.playerProfile.user_name,
          this.playerProfile.user_subtitle,
          this.playerProfile.user_prompt,
          this.playerProfile.info,
          this.playerProfile.system_prompt_example
        );
        if (!result?.success) {
          throw new Error("后端返回保存失败");
        }
        return true;
      } catch (e) {
        // 回滚失败写入；原错误继续向上抛，调用方可识别并展示具体原因
        this.playerProfile = snapshot;
        console.error("保存玩家档案失败:", e);
        throw e;
      }
    },

    /** 保存玩家头像 */
    async saveAvatar(imageBase64: string, ext?: string) {
      try {
        const res = await savePlayerAvatar(imageBase64, ext);
        this.playerProfile.avatar_path = res.avatar_path || this.playerProfile.avatar_path;
        return res;
      } catch (e) {
        console.error("保存玩家头像失败:", e);
        throw e;
      }
    },

    /** 更新玩家名 */
    setPlayerName(name: string) {
      this.playerProfile.user_name = name;
    },

    /** 更新玩家副标题 */
    setPlayerSubtitle(subtitle: string) {
      this.playerProfile.user_subtitle = subtitle;
    },
  },
});
