// 端末へ送った入力から「いま打っている行」を推定する。確かでなくなったら null(不明)にして、次の Enter まで候補を出さない。

const MAX_LINE_LENGTH = 4096;

// 空の行へ戻す入力。Enter・Ctrl+C・Ctrl+U は不明状態からも復帰する。
const RESET_CHARS = new Set(['\r', '\n', '\x03', '\x15']);

export class LineTracker {
  private buf = '';
  private unknown = false;

  /** 推定した行。不明なら null。 */
  get line(): string | null {
    return this.unknown ? null : this.buf;
  }

  /** terminal.onData で受け取った入力をそのまま渡す(1文字とは限らない)。 */
  feed(data: string): void {
    for (const ch of data) {
      if (RESET_CHARS.has(ch)) {
        this.buf = '';
        this.unknown = false;
        continue;
      }
      if (this.unknown) continue; // 不明のあいだは追跡しない(追加・削除しても null のまま)
      const cp = ch.codePointAt(0) ?? 0;
      if (cp === 0x7f || cp === 0x08) {
        // Backspace は最後の1文字を消す。空なら何もしない。
        this.buf = this.buf.slice(0, -1);
      } else if (cp >= 0x20 && cp <= 0x7e) {
        this.buf += ch;
        if (this.buf.length > MAX_LINE_LENGTH) this.unknown = true;
      } else {
        // それ以外の制御文字(Tab・Ctrl+R・ESC 始まりのエスケープ列など)と
        // 非 ASCII(IME 確定など)は追えないので不明にする。
        this.unknown = true;
      }
    }
  }

  /** 貼り付け・スニペット送信など、onData を通らない(または中身を追えない)入力があったとき。 */
  markUnknown(): void {
    this.unknown = true;
  }

  /** 新しい接続・再接続のとき。空の行から始める。 */
  reset(): void {
    this.buf = '';
    this.unknown = false;
  }
}

/** 画面のカーソル行の「カーソルより前」の文字列が、推定した行で終わっているか(= エコーで確認できたか)。 */
export function echoConfirmed(screenBeforeCursor: string, line: string): boolean {
  // line が空白で終わるときは画面側もその空白で終わっている必要がある(末尾を丸めて比較しない)。
  return screenBeforeCursor.endsWith(line);
}

/** カーソル行がパスワード等の入力欄に見えるか。見えるなら候補を出さない。 */
export function looksLikeSecretPrompt(screenLine: string): boolean {
  const s = screenLine.toLowerCase();
  return (
    s.includes('password') ||
    s.includes('passphrase') ||
    s.includes('passcode') ||
    s.includes('verification code') ||
    s.includes('[sudo]') ||
    /\bpin\b/.test(s) || // "spinning" 等の誤爆を避けるため単語境界で見る
    s.includes('パスワード') // i18n-ignore: 画面の文言ではなく照合用
  );
}
