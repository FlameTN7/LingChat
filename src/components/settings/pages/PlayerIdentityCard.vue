<template>
  <MenuItem :title="$t('settings.playerProfile.title')" size="small">
    <template #header>
      <UserRound :size="20" />
    </template>
    <div class="p-3 w-full">
      <!-- 玩家身份卡：像 AI 角色卡那样排布（头像 + 名称/副标题/简介 + 编辑按钮） -->
      <div
        class="group relative flex items-center rounded-2xl border border-white/20 bg-white/10 p-4 backdrop-blur-xl transition-all duration-300 hover:-translate-y-1 hover:border-amber-300/50 hover:shadow-2xl hover:shadow-amber-500/10"
      >
        <div
          class="text-brand absolute -top-2 -left-2 flex h-6 w-6 -rotate-18 transform items-center justify-center rounded-full shadow-md"
        >
          <Smile :size="20" />
        </div>

        <div class="flex w-28 shrink-0 flex-col items-center space-y-2 border-r border-white/10 pr-4 md:w-32">
          <div class="h-24 w-24 overflow-hidden rounded-full border-2 border-amber-300/50 shadow-lg">
            <img
              v-if="avatarUrl"
              :src="avatarUrl"
              :alt="playerName"
              class="h-full w-full object-cover transition-transform duration-500 group-hover:scale-110"
            />
            <div v-else class="flex h-full w-full items-center justify-center bg-white/10 text-white/40">
              <UserRound :size="40" />
            </div>
          </div>
          <span class="bg-amber-300 mt-1 h-1 w-6 rounded-full"></span>
          <h4 class="text-md text-center font-bold tracking-wide text-white drop-shadow-md">
            {{ playerName }}
          </h4>
          <span class="text-brand text-xs font-medium tracking-widest uppercase opacity-80">
            {{ playerSubtitle }}
          </span>
        </div>

        <div class="flex h-full min-h-36 flex-1 flex-col justify-between pl-4">
          <div class="pr-8">
            <p class="line-clamp-3 text-base leading-relaxed text-gray-200/90 opacity-80">
              {{ playerInfo || $t('settings.playerProfile.noInfo') }}
            </p>
          </div>
          <div class="mt-4 flex items-center justify-end gap-2">
            <button
              class="rounded-full border border-amber-300/40 bg-amber-400/80 px-5 py-1.5 text-xs font-bold text-slate-900 shadow-lg shadow-amber-500/20 transition-all hover:bg-amber-300"
              @click="openModal"
            >
              <span class="inline-flex items-center gap-1.5">
                <Pencil :size="14" />
                {{ $t('settings.playerProfile.edit') }}
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  </MenuItem>

  <!-- 玩家身份编辑弹窗 -->
  <PlayerProfileModal v-model:visible="modalVisible" :profile="localProfile" @saved="handleSaved" />
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Pencil, Smile, UserRound } from 'lucide-vue-next'
import { convertFileSrc } from '@tauri-apps/api/core'
import { MenuItem } from '../../ui'
import { useUserStore } from '../../../stores/modules/user/user'
import type { PlayerProfile } from '../../../api/services/game-info'
import PlayerProfileModal from './PlayerProfileModal.vue'

const { t } = useI18n()
const userStore = useUserStore()

const modalVisible = ref(false)
/** 本地玩家档案副本（用于编辑弹窗） */
const localProfile = ref<PlayerProfile>({
  user_name: '玩家',
  user_subtitle: '',
  user_prompt: '',
  info: '',
  system_prompt_example: '',
  avatar_path: null,
})

const playerName = computed(() => localProfile.value.user_name || '玩家')
const playerSubtitle = computed(() => localProfile.value.user_subtitle || '')
const playerInfo = computed(() => localProfile.value.info || '')
const avatarUrl = computed(() =>
  localProfile.value.avatar_path ? convertFileSrc(localProfile.value.avatar_path) : '',
)

async function loadProfile() {
  await userStore.loadPlayerProfile()
  localProfile.value = { ...userStore.playerProfile }
}

function openModal() {
  localProfile.value = { ...userStore.playerProfile }
  modalVisible.value = true
}

function handleSaved() {
  localProfile.value = { ...userStore.playerProfile }
}

onMounted(loadProfile)

// 当 user store 更新时同步展示（如初始化数据已加载）
watch(
  () => userStore.playerProfile,
  (profile) => {
    localProfile.value = { ...profile }
  },
  { deep: true },
)
</script>
