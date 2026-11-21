// 端末へ送った入力から「いま打っている行」を推定する。確かでなくなったら null(不明)にして、次の Enter まで候補を出さない。

export class LineTracker {
  /** 推定した行。不明なら null。 */
  get line(): string | null {
    return null;
  }

  /** terminal.onData で受け取った入力をそのまま渡す。 */
  feed(data: string): void {
    void data;
  }

  /** 貼り付け・スニペット送信など、onData を通らない(または中身を追えない)入力があったとき。 */
  markUnknown(): void {}

  /** 新しい接続・再接続のとき。空の行から始める。 */
  reset(): void {}
}

/** 画面のカーソル行の「カーソルより前」の文字列が、推定した行で終わっているか(= エコーで確認できたか)。 */
export function echoConfirmed(screenBeforeCursor: string, line: string): boolean {
  void screenBeforeCursor;
  void line;
  return false;
}

/** カーソル行がパスワード等の入力欄に見えるか。見えるなら候補を出さない。 */
export function looksLikeSecretPrompt(screenLine: string): boolean {
  void screenLine;
  return true;
}
