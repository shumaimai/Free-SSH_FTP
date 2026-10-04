# 端末・AI拡張の監査対応と検証手順

## 監査の対象

2026-10-02のIssue #151〜#160の監査コメントは、実装前の
`22342bad481390864faf7d97003e9747cf048cc4`を対象としている。
「機能が存在しない」という記述はその時点のmainについて正しい。
実装は未マージのPR #161〜#169にある。再監査は該当PRのheadを対象とする。
全機能をまとめて見る場合は`feature/claude-code`を使う。
2026-10-04の再監査で指摘された、長いツール結果のJSON破損をこのブランチで修正した。
初期の子PRだけでは後続のUI/API/自動導入の改修が見えないため、全体の再確認はPR #169を対象にする。
mainwindow.pyをサイズ制限で省略する場合は、下表の連携箇所を個別に読む。

| Issue / PR | 責務と実装の確認先 | 主な自動検証 |
|---|---|---|
| #152 / #161 | `terminal_backend.py`, `session_registry.py`, `terminal_binding.py`; `TerminalWidget.attach/detach`, `SessionTab`のBinding | `test_terminal_backend.py`, `test_terminal_keys.py`, `test_terminal_resize.py`, `test_terminal_more.py` |
| #153 / #162 | `local_terminal.py`; `AppWindow.open_local_terminal` | `test_local_terminal.py`の模擬Backend、Windows実ConPTY |
| #154 / #163 | `SessionTab._toggle_terminal_panes`, `active_terminal`, `reconnect_session`, `shutdown` | `test_mainwindow_windows.py`, `test_config.py` |
| #155 / #164 | `command_broker.py`, `ssh_core.command_channel` | `test_command_broker.py`, `test_ssh_core.py`。実Paramikoの出力/終了コード、ACK未応答の締切/停止 |
| #156 / #165 | `ai_core.py`, `ai_panel.py`; `AppWindow`のAIドックと終了待ち | `test_ai_core.py`。対象限定、マスク、取消、GUIの共有設定 |
| #157 / #166 | `ai_api.py`, `ai_http.py`, `ai_settings.py`, `ai_secrets.py` | `test_ai_api.py`。模擬ストリーム、設定再読込、SSH書き出し/同期バンドルへの秘密の非混入 |
| #158 / #167 | `chatgpt_oauth.py`, `chatgpt_dialog.py` | `test_chatgpt_oauth.py`。実loopback、模擬issuer、RSA署名、state/nonce/aud/期限、更新直列化、失効失敗 |
| #159 / #168 | `mcp_bridge.py`, `mcp_stdio.py`, `tools/hashi_mcp.py`, `Hashi.spec` | `test_mcp_bridge.py`。実stdio/IPC、認証・別インスタンス拒否、取消、凍結ヘルパー |
| #160 / #169 | `claude_cli.py`, `claude_install.py`; `OfficialCliPage`, `AppWindow.open_official_cli` | `test_claude_cli.py`, `test_claude_install.py`。起動オプション、未改変の公式導入、取消、認証画面の非共有/非ログ/秘密の非自動送信 |

#152は契約とSSHアダプターまで、ConPTY実装は#153、表示構成は#154で扱う。
端末ID/世代の正はSessionRegistry、操作ID/承認/取消の正はCommandBrokerであり、
組込みAIとMCPで別の実行管理を作らない。親#151は進捗管理、子Issueは個別の完了条件を持つ。
改善Issueのため、不具合報告用の再現情報の欠如をもって未対応とは扱わない。

## 照合して修正した点

- CIの`ruff check . || true`を廃止し、失敗を検出する。headless用のkeyring設定を明示する。
- `Settings.DEFAULTS`へ`local_terminal_start_dir`（空はホーム）、`terminal_dual_pane`
  （既定OFF）、`ai_api_kind`、`ai_api_profiles`を登録し、再起動で設定が消える問題を修正する。
  ファイル2ペインの`dual_pane`/`local_start_dir`とは別設定。
- 既存SSHの`exec_command`/`run_sudo`で、出力/終了待ちに全体の締切を適用する。
  受信失敗から終了待ちへ進まず、例外でも専用チャネルを閉じる。
  AI/MCPの独立SSHも、exec要求のACK待ちを締切/取消で解除する。
  サーバー側のプロセス終了は保証しない。チャネル開始中の取消は開始のtimeout以内に検出する。
- SSHのシェルはPTY名から推測せず`unknown`を公開する。独立execは本文をサーバーへ送り、
  対話PTYのcwd/環境を引き継がない。cwdは検出していないため`null`。
- 互換APIには相談専用の選択肢を設ける。toolsを送らず、応答にツール要求があっても実行しない。
  Chat Completions / Responses / Messagesの形式を選択できる。HTTPエラー後の自動POST再送はしない。
- Windows CIにAI/API/OAuth/設定/SSHの検証を追加し、実行環境とEXEサイズを記録する。
- 全角文字が右端に収まらない場合は次行へ移し、半角で片側だけ上書きした全角セルを正規化する。
  行消去時に古い折返し余白を解除し、再リサイズで実際の空白が消える問題を修正する。
- 保存キーの読込みもQtワーカーへ移し、読込み中のGUI応答と終了待ちを検証する。
  接続確認はツール利用の設定に関係なくtoolsを送らず、設定値は維持する。
- Claude Codeの公式スクリプト取得中にキャンセルされた場合は、導入プロセスを開始しない。

## コンテキスト・認証・保存の確定仕様

出力は台帳の直近出力と現在画面を別々に保持し、それぞれ64 Ki文字まで。
初回の送信プレビューは各端末から各8,000文字、JSON全体はUTF-8で64,000バイトまで。
追加の`read_output`は各出力/画面16,000文字まで。既知の秘密パターンをマスクするが、
完全検出は保証しないため、送信前に編集できる。cwdや対話入力の完了を推測しない。
ツール結果はUTF-8で32,000バイトまで。`observation_json`で個々の文字列をマスク・短縮してから
JSONを生成し、シリアライズ済みJSONの末尾を切らない。省略時は`truncated: true`を付ける。
大きなトップレベル一覧は`items`と`truncated`を持つオブジェクトに変換する。
端末ID・操作IDを短縮せず、通常の結果では接続世代・状態・終了コードも保持する。
会話と操作履歴はメモリのみで、会話の上限はシリアライズ後256,000文字、1回答の操作巡回は8回まで。
接続方式変更時は会話を消去し、アプリ終了で破棄する。共有/許可も保存せず15分で失効する。

APIのHTTP処理は標準urllibで、認証付きリダイレクトは禁止。OAuth署名検証はPyJWT/cryptography。
キー/ChatGPTトークンは`Hashi.AI`のkeyringまたは専用Fernetファイルへ保存し、
SSHのCredentialStore、設定JSON、接続情報の書き出し/P2P/クラウド同期から分離する。
APIキーは方式と接続URLごと、OAuthは発行client_idごとに分ける。APIキーの保存は明示選択。
ChatGPTのサインアウトは遠隔失効を試みてローカルトークンを削除し、失効未確認なら通知する。
callbackは`127.0.0.1`の空きポートの`/auth/callback`、待機期限180秒。
利用可能プラン/モデルはアカウントの許可scopeと公式モデル一覧で確認する。

公式Claude CodeはHashiの資格情報ストアを使わず、本人のログイン/請求に任せる。
専用CLI端末はRegistry/Bindingへ登録せず、HashiのSessionLogも接続しない。
パスワードプロンプトの表示だけでは秘密を送信しない。CLI自身の履歴はCLI側の設定に従う。
導入済みWindowsネイティブ版の2.0.0以上と必要フラグを検査する。実際の対応可否は
`--help`も確認し、npm/WSL版の埋込み起動やバイナリ同梱は行わない。

ConPTYはWindows 10 1809以降、pywinpty 3.0.5以上をWindows限定で導入し、
pywinptyの[MITライセンス](https://github.com/andfoy/pywinpty/blob/main/LICENSE.txt)を適用する。
ConPTYのUnicode出力をUTF-8 bytesにして既存TerminalWidgetへ渡す。通常のローカルCMDも
現在はSessionLogを接続しない。終了は生成したPIDのツリーに限定する。
終了時はpywinpty内部readerもsocket shutdown/native cancel_io/joinで回収する。
インタラクティブCMDはHashi起動時の環境を引き継ぐ。環境変数の編集UIとシェルcwd検出は未実装。
他の端末の変更済み環境を引き継いだとは表示しない。

## Windows・実サービスでの確認手順（未実施分）

CIのWindows runner/模擬API検証は、利用者のWindowsデスクトップ・実サービスでの検証を代替しない。
手動実施時はOSビルド、DPI（100/125/150%）、Python/依存の版、PR head SHA、
実CLI版、SSHサーバーOS/sshd版、API接続先とモデルを記録する。秘密/認証URLは添付しない。

| 対象 | 手順と合格条件 |
|---|---|
| CMD / W端末 | Hashi.exeから単独CMDとW端末を開き、日本語IME・全角・コピー・貼付・リサイズ・TUIを確認。長い処理にCtrl+Cを送り、通常exit・タブ終了との違いを確認。SSH再接続とCMD表示切替で同じCMDが続くこと、タブ閉鎖で所有プロセスと受信スレッドが残らないことを確認 |
| 操作先 / 停止 | SSH/CMDを交互にフォーカスし、スニペットの送信先、SSHパスワード/ログの操作先を確認。別端末/古い世代/共有期限切れへの操作を拒否。承認待ち・独立実行中・MCPクライアント異常終了で停止し、完了不明を成功と表示しない |
| 実API | 各方式で逐次回答、tool往復、認証エラー、利用上限、オフライン、途中切断を確認。接続先変更でキーが消え、保存キー削除後に復元されないことを確認。互換方式はtoolsなしの相談と対応モデルのtoolsありを分けて確認 |
| 実ChatGPT | 公式ブラウザで初回ログイン、拒否/取消、再認証、2アカウントの切替、期限後更新、サインアウトを確認。モデル一覧をアカウント別に取得し、過去アカウントのトークン/会話を使わないこと、遠隔失効失敗の通知を確認 |
| 実Claude Code | 本人の公式CLIでログイン、会話、再開、TUI/リサイズ/Ctrl+Cを確認。Hashi MCPで指定したCMD/SSHへ操作し、Hashiの承認と操作履歴を確認。CLI認証画面は共有/Hashiログに含まれず、内蔵ツール/フックの許可はHashi Brokerとは別であることを確認 |
| 配布・負荷 | Hashi.exe/HashiMCP.exeを空白/日本語入りフォルダへ置き起動。CLI未導入の案内も確認。1/10/20端末で起動時間・GUI応答・RSS・スレッド数を測り、閉鎖後にプロセス/スレッド/接続ファイルが残らないことを確認。現時点で性能値/許容閾値は未確定 |

Windows CIは実ConPTY、Unicode/Ctrl+C、独立CMDの出力/終了コード/timeout、2つのEXEビルドと
凍結ヘルパーIPCを確認する。API応答とOAuth issuerは模擬であり、実課金/実ログインではない。
任意の外部SSHサーバー、実Claude Code認証/TUI、利用者デスクトップ、負荷測定は上表の未検証分として残す。
公開時のサービス規約の確認は[使い方](terminal-ai.md)の公式資料を参照する。

## 再監査の依頼単位

PRは#161→#169の依存順で確認する。基盤変更は後続へ通常のmergeで反映し、履歴を強制更新しない。
監査にはPRのhead SHAと対象ファイルを添え、確認したコード、模擬応答、実接続、実アカウントを区別する。
全体の安全性確認済みとは扱わず、手動完了条件が残るIssue/PRはオープン/Draftを維持する。
