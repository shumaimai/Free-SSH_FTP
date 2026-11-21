// 端末の onData を通らない入力（スニペット送信など）を、開いている端末へ知らせる小さな仕掛け。
// コマンド補完の行推定(LineTracker)が「中身を追えない入力があった」と分かるようにするためのもので、
// 送った本文そのものは渡さない（送り先のセッションIDだけ）。
const handlers = new Set<(sessionId: string) => void>();

/** onData を通らずに端末へ入力を送ったあとに呼ぶ。 */
export function notifyExternalInput(sessionId: string): void {
  handlers.forEach((handler) => handler(sessionId));
}

/** 端末側が登録する。戻り値の関数で解除する。 */
export function onExternalInput(handler: (sessionId: string) => void): () => void {
  handlers.add(handler);
  return () => handlers.delete(handler);
}
