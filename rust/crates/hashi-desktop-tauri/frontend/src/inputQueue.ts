// 端末入力を順番どおりに送る。IPCの呼び出しは追い越し得るので、送信中に溜まった分は1回にまとめる。
export class InputQueue {
  private buffer = '';
  private sending = false;

  constructor(
    private readonly send: (data: string) => Promise<void>,
    private readonly onError: (message: string) => void,
  ) {}

  push(data: string) {
    this.buffer += data;
    void this.drain();
  }

  private async drain() {
    if (this.sending) return;
    this.sending = true;
    try {
      while (this.buffer.length > 0) {
        // Rust側の上限(1MiB)に収める。文字列を切る位置はサロゲートペアを割らない。
        let end = Math.min(this.buffer.length, 256 * 1024);
        const code = this.buffer.charCodeAt(end - 1);
        if (end < this.buffer.length && code >= 0xd800 && code <= 0xdbff) end -= 1;
        const chunk = this.buffer.slice(0, end);
        this.buffer = this.buffer.slice(end);
        await this.send(chunk);
      }
    } catch (error) {
      this.buffer = '';
      this.onError(String(error));
    } finally {
      this.sending = false;
    }
  }
}
