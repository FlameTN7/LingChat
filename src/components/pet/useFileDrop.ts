// 文件投喂相关逻辑

import { ref, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useUIStore } from "@/stores/modules/ui/ui";
import { invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { isLinux } from "@/utils/platform";

const IMAGE_EXTS = new Set([".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]);

const UNSUPPORTED_FILE = "不支持的文件类型";

function isImageFile(path: string): boolean {
  return IMAGE_EXTS.has(path.slice(path.lastIndexOf(".")).toLowerCase());
}

async function feedTextFile(path: string): Promise<void> {
  if (isLinux()) return invoke("feed_text_file", { path });

  let text: string;
  try {
    const bytes = await readFile(path);
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(UNSUPPORTED_FILE);
  }
  await invoke("feed_text", { text });
}

export function useFileDrop() {
  const isDragging = ref(false);
  const hasFile = ref(false);
  const ui = useUIStore();
  let unlisten: (() => void) | null = null;

  onMounted(async () => {
    unlisten = await getCurrentWindow().onDragDropEvent(async (event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") {
        isDragging.value = true;
        return;
      }

      if (event.payload.type === "leave") {
        isDragging.value = false;
        return;
      }

      // drop
      isDragging.value = false;
      const paths = event.payload.paths;

      if (paths.length > 1) {
        ui.showNotification({
          type: "error",
          title: "不支持多个文件",
          message: "呜啊！不要一次塞这么多啦~",
          duration: 2500,
          skipTipsCheck: true,
        });
        return;
      }

      const path = paths[0];

      if (!path) return;
      const image = isImageFile(path);
      hasFile.value = true;
      try {
        if (image) await invoke("feed_image", { path });
        else await feedTextFile(path);
        ui.showNotification({
          type: "success",
          title: image ? "投喂成功！" : "文本投喂成功！",
          duration: 2000,
          skipTipsCheck: true,
        });
      } catch (error) {
        hasFile.value = false;
        if (!image && String(error).includes(UNSUPPORTED_FILE)) {
          ui.showNotification({
            type: "error",
            title: UNSUPPORTED_FILE,
            message: "呜啊！这个文件我看不懂啦~",
            duration: 2500,
            skipTipsCheck: true,
          });
        } else {
          console.error("投喂失败:", error);
        }
      }
    });
  });

  onUnmounted(() => {
    unlisten?.();
  });

  return { isDragging, hasFile };
}
