// コマンド補完の型。エンジン(engine.ts)・行の推定(lineTracker.ts)・画面(TerminalView.tsx)の間の約束。
// このフォルダのファイルは React・Vite・i18n.ts を import しない(node --test でそのまま動かすため)。

/** 候補の辞書を選ぶシェルの種類。SSH 先は posix、Windows のローカル端末(CMD)は cmd。 */
export type ShellKind = 'posix' | 'cmd';

export interface Candidate {
  /** 補完したあとのトークン全体(例: `restart`)。一覧に表示する。 */
  word: string;
  /** 端末へ送る文字列 = まだ打っていない残り(例: `res` まで打っていれば `tart`)。削除文字は使わない。 */
  insert: string;
  /** 短い説明。日本語の原文で、画面では t() を通して表示する。無ければ空文字。 */
  description: string;
}

export interface CompletionResult {
  /** 灰色で出す続き(= items[0].insert)。出さないときは null。 */
  ghost: string | null;
  /** Ctrl+Space の一覧に出す候補。並びは辞書の優先順。 */
  items: Candidate[];
}
