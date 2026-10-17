// 画面の例外をRust側のログへ送る。送れない（試験用の偽の実装・起動前）ときは何もしない。
import { invoke } from '@tauri-apps/api/core';

export function reportClientError(message: string): void {
  try {
    void invoke('client_log', { message }).catch(() => {});
  } catch {
    // 画面の外へ送れない環境では、記録をあきらめる。
  }
}
