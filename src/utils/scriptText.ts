/**
 * 剧本台词正文清理工具（共享）。
 *
 * 剧本引擎把旁白整行存储为 `{旁白: ...}`（剧情演绎提示为 `{旁白: （接下来的剧情演绎提示：...）}`）。
 * 直接 `replace(/\{[\s\S]*?\}/g, '')` 会把整行旁白连正文一起删掉，导致读档/历史里旁白显示为空白。
 * 这里统一处理：整行旁白包裹时去掉外层保留正文，仅剥离内联 `{..}` 块；剧情演绎提示行整行过滤。
 */

/** 清理台词正文：整行 `{旁白: ...}` 包裹时去掉外层包裹保留正文；否则剥离内联 `{..}` 块。 */
export function cleanLineContent(raw: string): string {
  if (raw.startsWith('{旁白:') && raw.endsWith('}')) {
    return raw.slice('{旁白:'.length, -1).trim()
  }
  return raw.replace(/\{[\s\S]*?\}/g, '').trim()
}

/** 是否为 AI 剧情演绎提示行（剧本内嵌 prompt，实玩时也不显示，读档历史保持一致） */
export function isPlotPromptLine(raw: string): boolean {
  if (raw.startsWith('{旁白:') && raw.endsWith('}')) {
    const inner = raw.slice('{旁白:'.length, -1).trim()
    return inner.startsWith('（接下来的剧情演绎提示')
  }
  return false
}
