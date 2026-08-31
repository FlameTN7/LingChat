<template>
  <Transition name="modal">
    <div
      v-if="visible"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm"
      @click="handleClose"
    >
      <div
        class="flex h-[85dvh] w-full max-w-4xl flex-col overflow-hidden rounded-3xl border
          border-white/20
          bg-[linear-gradient(135deg,rgba(255,255,255,0.15)_0%,rgba(255,255,255,0.05)_100%)]
          text-white shadow-[0_20px_60px_rgba(0,0,0,0.4),inset_0_0_1px_rgba(255,255,255,0.3)]
          backdrop-blur-[30px] backdrop-saturate-180"
        @click.stop
      >
        <!-- Header -->
        <div
          class="flex items-center justify-between border-b border-white/10
            bg-[linear-gradient(180deg,rgba(255,255,255,0.1)_0%,rgba(255,255,255,0.05)_100%)] p-6"
        >
          <div class="flex items-center gap-4">
            <div
              class="flex h-12 w-12 items-center justify-center rounded-xl bg-white/10 shadow-inner"
            >
              <Icon icon="setting" />
            </div>
            <div>
              <h2 class="m-0 text-xl font-bold drop-shadow-[0_2px_4px_rgba(0,0,0,0.3)]">
                {{ $t("settings.playerProfile.modalTitle") }}
              </h2>
              <p class="m-0 text-sm text-white/50">
                {{ $t("settings.playerProfile.modalSubtitle") }}
              </p>
            </div>
          </div>
          <button
            class="flex h-9 w-9 cursor-pointer items-center justify-center rounded-full border-none
              bg-white/10 text-white transition-all duration-200 hover:rotate-90 hover:bg-white/20"
            @click="handleClose"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </div>

        <!-- Content -->
        <div class="flex flex-1 flex-row overflow-hidden">
          <!-- Sidebar -->
          <div
            class="tab-sidebar-scroll flex w-44 shrink-0 flex-col gap-2 overflow-y-auto border-r
              border-white/10 bg-black/10 p-3"
          >
            <button
              v-for="tab in tabs"
              :key="tab.id"
              class="w-full cursor-pointer rounded-xl border-none bg-transparent px-4 py-2.5
                text-left font-medium text-white/60 transition-all duration-200 hover:bg-white/5
                hover:text-white"
              :class="{
                'bg-[rgba(94,114,228,0.2)] font-semibold! text-[#79d9ff]!': activeTab === tab.id,
              }"
              @click="activeTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </div>

          <!-- Tab Panels -->
          <div class="relative flex-1 overflow-y-auto p-6">
            <!-- 基础 tab：玩家名 / 副标题 / 简介 -->
            <div v-if="activeTab === 'basic'" class="mx-auto max-w-3xl space-y-4">
              <div class="flex flex-col gap-2">
                <label class="text-[13px] font-medium text-white/60">{{
                  $t("settings.playerProfile.userName")
                }}</label>
                <input
                  v-model="form.user_name"
                  type="text"
                  :placeholder="$t('settings.playerProfile.userNamePlaceholder')"
                  class="form-control rounded-xl border border-white/10 bg-black/20 px-3.5 py-2.5
                    text-sm text-white transition-all duration-200 outline-none"
                />
              </div>
              <div class="flex flex-col gap-2">
                <label class="text-[13px] font-medium text-white/60">{{
                  $t("settings.playerProfile.userSubtitle")
                }}</label>
                <input
                  v-model="form.user_subtitle"
                  type="text"
                  :placeholder="$t('settings.playerProfile.userSubtitlePlaceholder')"
                  class="form-control rounded-xl border border-white/10 bg-black/20 px-3.5 py-2.5
                    text-sm text-white transition-all duration-200 outline-none"
                />
              </div>
              <div class="flex flex-col gap-2">
                <label class="text-[13px] font-medium text-white/60">{{
                  $t("settings.playerProfile.playerInfo")
                }}</label>
                <textarea
                  v-model="form.info"
                  rows="4"
                  :placeholder="$t('settings.playerProfile.playerInfoPlaceholder')"
                  class="form-control resize-none rounded-xl border border-white/10 bg-black/20
                    px-3.5 py-2.5 text-sm leading-relaxed text-white transition-all duration-200
                    outline-none"
                ></textarea>
              </div>
            </div>

            <!-- 头像 tab：一张静态图 -->
            <div v-else-if="activeTab === 'avatar'" class="mx-auto max-w-3xl space-y-5">
              <div
                class="rounded-xl border px-4 py-3"
                :class="
                  avatarFile || avatarPreviewUrl
                    ? 'border-emerald-400/40 bg-emerald-300/10'
                    : 'border-rose-400/40 bg-rose-300/10'
                "
              >
                <div class="text-sm font-medium">
                  {{ $t("settings.playerProfile.avatar.status") }}
                </div>
              </div>

              <label
                class="flex cursor-pointer flex-col gap-2 rounded-2xl border p-2 transition"
                :class="
                  avatarFile || avatarPreviewUrl
                    ? 'border-emerald-400/50 bg-emerald-300/10'
                    : 'border-rose-400/50 bg-white/5 hover:bg-white/10'
                "
                @dragover.prevent="dragOver = true"
                @dragleave.prevent="dragOver = false"
                @drop.prevent="onDrop"
              >
                <div class="flex justify-between text-xs text-white/80">
                  <span>{{ $t("settings.playerProfile.avatar.label") }}</span>
                  <span>{{
                    avatarFile
                      ? $t("settings.playerProfile.avatar.uploaded")
                      : $t("settings.playerProfile.avatar.notUploaded")
                  }}</span>
                </div>
                <div
                  class="aspect-square overflow-hidden rounded-xl border border-white/10
                    bg-slate-900/60"
                >
                  <img
                    v-if="avatarPreviewUrl"
                    :src="avatarPreviewUrl"
                    alt="avatar preview"
                    class="h-full w-full object-cover"
                  />
                  <div v-else-if="avatarUrl" class="h-full w-full">
                    <img :src="avatarUrl" alt="current avatar" class="h-full w-full object-cover" />
                  </div>
                  <div
                    v-else
                    class="flex h-full w-full items-center justify-center text-xs text-white/40"
                  >
                    {{ $t("settings.playerProfile.avatar.dropHint") }}
                  </div>
                </div>
                <input type="file" accept="image/*" class="hidden" @change="onFileChange" />
              </label>
            </div>

            <!-- 设定 tab：人格设定 / 说话示例 -->
            <div v-else-if="activeTab === 'prompts'" class="mx-auto max-w-3xl space-y-4">
              <div class="flex flex-col gap-2">
                <label class="text-[13px] font-medium text-white/60">{{
                  $t("settings.playerProfile.userPrompt")
                }}</label>
                <textarea
                  v-model="form.user_prompt"
                  rows="10"
                  :placeholder="$t('settings.playerProfile.userPromptPlaceholder')"
                  class="form-control resize-none rounded-xl border border-white/10 bg-black/20
                    px-3.5 py-2.5 font-mono text-sm leading-relaxed text-white transition-all
                    duration-200 outline-none"
                ></textarea>
                <p class="text-[0.68rem] leading-[1.6] text-white/40">
                  {{ $t("settings.playerProfile.userPromptHint") }}
                </p>
              </div>
              <div class="flex flex-col gap-2">
                <label class="text-[13px] font-medium text-white/60">{{
                  $t("settings.playerProfile.promptExample")
                }}</label>
                <textarea
                  v-model="form.system_prompt_example"
                  rows="6"
                  :placeholder="$t('settings.playerProfile.promptExamplePlaceholder')"
                  class="form-control resize-none rounded-xl border border-white/10 bg-black/20
                    px-3.5 py-2.5 font-mono text-sm leading-relaxed text-white transition-all
                    duration-200 outline-none"
                ></textarea>
              </div>
            </div>
          </div>
        </div>

        <!-- Footer -->
        <div
          class="flex justify-end gap-3 border-t border-white/10
            bg-[linear-gradient(180deg,rgba(255,255,255,0.05)_0%,rgba(255,255,255,0.1)_100%)] p-4"
        >
          <button
            class="cursor-pointer rounded-[20px] border-none bg-white/10 px-5 py-2 text-sm
              font-medium text-white transition-all duration-200 hover:bg-white/20"
            @click="handleClose"
          >
            {{ $t("settings.playerProfile.cancel") }}
          </button>
          <button
            class="cursor-pointer rounded-[20px] border-none bg-[#5e72e4] px-5 py-2 text-sm
              font-medium text-white transition-all duration-200 hover:enabled:-translate-y-px
              hover:enabled:bg-[#4a5acf] hover:enabled:shadow-[0_4px_12px_rgba(94,114,228,0.3)]
              disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="saving"
            @click="saveSettings"
          >
            <span
              v-if="saving"
              class="mr-2 inline-block h-3.5 w-3.5 animate-spin rounded-full border-2
                border-white/30 border-t-white"
            ></span>
            {{ saving ? $t("settings.playerProfile.saving") : $t("settings.playerProfile.save") }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
  import { computed, onUnmounted, ref, watch } from "vue";
  import { useI18n } from "vue-i18n";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { Icon } from "../../base";
  import { useUserStore } from "../../../stores/modules/user/user";
  import type { PlayerProfile } from "../../../api/services/game-info";

  const props = defineProps<{
    visible: boolean;
    profile: PlayerProfile;
  }>();

  const emit = defineEmits<{
    (e: "update:visible", value: boolean): void;
    (e: "saved"): void;
  }>();

  const { t } = useI18n();
  const userStore = useUserStore();

  const activeTab = ref("basic");
  const saving = ref(false);

  // 本地表单副本
  const form = ref<PlayerProfile>({
    user_name: "玩家",
    user_subtitle: "",
    user_prompt: "",
    info: "",
    system_prompt_example: "",
    avatar_path: null,
  });

  // 头像上传态
  const avatarFile = ref<File | null>(null);
  const avatarPreviewUrl = ref("");
  const dragOver = ref(false);

  const tabs = computed(() => [
    { id: "basic", label: t("settings.playerProfile.tabs.basic") },
    { id: "avatar", label: t("settings.playerProfile.tabs.avatar") },
    { id: "prompts", label: t("settings.playerProfile.tabs.prompts") },
  ]);

  const avatarUrl = computed(() =>
    form.value.avatar_path ? convertFileSrc(form.value.avatar_path) : ""
  );

  // 打开弹窗时同步表单
  watch(
    () => props.visible,
    (visible) => {
      if (visible) {
        form.value = { ...props.profile };
        resetAvatar();
      }
    }
  );

  function resetAvatar() {
    if (avatarPreviewUrl.value) URL.revokeObjectURL(avatarPreviewUrl.value);
    avatarPreviewUrl.value = "";
    avatarFile.value = null;
    dragOver.value = false;
  }

  const handleClose = () => {
    if (saving.value) return;
    emit("update:visible", false);
  };

  function onFileChange(event: Event) {
    const target = event.target as HTMLInputElement;
    const file = target.files?.[0];
    if (!file) return;
    avatarFile.value = file;
    setPreview(file);
  }

  function onDrop(event: DragEvent) {
    dragOver.value = false;
    const file = event.dataTransfer?.files?.[0];
    if (!file || !file.type.startsWith("image/")) return;
    avatarFile.value = file;
    setPreview(file);
  }

  function setPreview(file: File) {
    if (avatarPreviewUrl.value) URL.revokeObjectURL(avatarPreviewUrl.value);
    avatarPreviewUrl.value = URL.createObjectURL(file);
  }

  async function saveSettings() {
    saving.value = true;
    try {
      // 1. 保存文本字段
      const ok = await userStore.savePlayerProfile({
        user_name: form.value.user_name.trim() || "玩家",
        user_subtitle: form.value.user_subtitle.trim(),
        user_prompt: form.value.user_prompt,
        info: form.value.info,
        system_prompt_example: form.value.system_prompt_example,
      });

      // 2. 若有新头像，先上传头像（写保存后才拿到 avatar_path）
      if (avatarFile.value) {
        const ext = avatarFile.value.name.split(".").pop() || "png";
        const imageBase64 = await readFileAsBase64(avatarFile.value);
        await userStore.saveAvatar(imageBase64, ext);
      }

      if (ok) {
        // 同步更新弹窗表单的 avatar_path（保存头像后 userStore 已更新）
        form.value = { ...userStore.playerProfile };
        emit("saved");
        emit("update:visible", false);
      }
    } catch (e) {
      console.error("保存玩家档案失败:", e);
    } finally {
      saving.value = false;
    }
  }

  function readFileAsBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve((reader.result as string).split(",")[1] || "");
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  }

  onUnmounted(() => {
    if (avatarPreviewUrl.value) URL.revokeObjectURL(avatarPreviewUrl.value);
  });
</script>

<style scoped>
  /* 竖向侧边栏：细滚动条 */
  .tab-sidebar-scroll {
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.18) transparent;
  }
  .tab-sidebar-scroll::-webkit-scrollbar {
    width: 6px;
  }
  .tab-sidebar-scroll::-webkit-scrollbar-track {
    background: transparent;
  }
  .tab-sidebar-scroll::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.18);
    border-radius: 3px;
  }
  .tab-sidebar-scroll::-webkit-scrollbar-thumb:hover {
    background: rgba(255, 255, 255, 0.32);
  }
  .form-control:focus {
    border-color: #79d9ff;
    background: rgba(0, 0, 0, 0.3);
    box-shadow: 0 0 0 3px rgba(121, 217, 255, 0.2);
  }

  .modal-enter-active,
  .modal-leave-active {
    transition: all 0.25s ease;
  }
  .modal-enter-from,
  .modal-leave-to {
    opacity: 0;
    transform: translateY(8px);
  }
</style>
