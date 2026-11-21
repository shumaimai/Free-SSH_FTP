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
      if (cp === 0x00) {
        // Ctrl+Space(NUL)は一覧を開けなかったときにシェルへ素通しされる。readline では set-mark、
        // CMD では何もしないので、行の中身は変わらない。不明にしない(画面が変わればエコー確認で止まる)。
        continue;
      }
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

// 端末(xterm.js)が問い合わせに自動で答えた報告も onData に出る。打った文字ではないので行の推定に入れない。
// フォーカスの出入り(CSI I / CSI O)、端末の種類(DA1/DA2)、モードの問い合わせ(DECRPM)、色などの OSC の返事。
// どれもキー1つでは送られない形なので、1回の onData がまるごとこの形のときだけ報告とみなす。
const TERMINAL_REPORT_RE = /^(?:\x1b\[[IO]|\x1b\[[?>][\d;]*c|\x1b\[\??[\d;]*\$y|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\))$/;

/** 端末が自動で送った報告か(打った文字ではない)。 */
export function isTerminalReport(data: string): boolean {
  return TERMINAL_REPORT_RE.test(data);
}

/**
 * カーソル位置の報告(CSI 行;列 R)の形か。Shift+F3 など修飾キー付きの F3 も同じ形(CSI 1;2 R)になるので、
 * 呼び出し側は「位置を問い合わせられた(CSI 6 n)直後」に限って報告とみなすこと。
 */
export function isCursorReport(data: string): boolean {
  return /^\x1b\[\d+;\d+R$/.test(data);
}

/** 画面のカーソル行の「カーソルより前」の文字列が、推定した行で終わっているか(= エコーで確認できたか)。 */
export function echoConfirmed(screenBeforeCursor: string, line: string): boolean {
  // line が空白で終わるときは画面側もその空白で終わっている必要がある(末尾を丸めて比較しない)。
  return screenBeforeCursor.endsWith(line);
}

// [sudo] 以外の語は「プロンプト部分の末尾」(`:` や `?` で終わる)に出るときだけ疑う。
// `u@h:~/password-tool$ ` のようなパス名での誤爆を避けるため。
const SECRET_TAIL_RE = /(?:password|passphrase|passcode|verification code|secret|token|otp|\bpin\b|パスワード)[^$#%>]*[:?？：]\s*$/i; // i18n-ignore: 画面の文言ではなく照合用

/** カーソル行がパスワード等の入力欄に見えるか。見えるなら候補を出さない。 */
export function looksLikeSecretPrompt(screenLine: string): boolean {
  return screenLine.toLowerCase().includes('[sudo]') || SECRET_TAIL_RE.test(screenLine);
}
