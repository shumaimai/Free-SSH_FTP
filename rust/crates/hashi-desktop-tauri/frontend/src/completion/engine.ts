// 組み込みのコマンド辞書から、入力中の行の補完候補を作る。
import type { CompletionResult, ShellKind } from './types.ts';

/** 入力中の行(プロンプトを除いた、カーソルまでの文字列)に対する候補。 */
export function complete(shell: ShellKind, line: string, limit = 50): CompletionResult {
  void shell;
  void line;
  void limit;
  return { ghost: null, items: [] };
}
