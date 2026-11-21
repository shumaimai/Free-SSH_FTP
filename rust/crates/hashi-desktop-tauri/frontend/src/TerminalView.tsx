import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import {
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  makeStyles,
  mergeClasses,
  tokens,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { SessionInfo } from './types';
import type { Candidate, ShellKind } from './completion/types.ts';
import { complete } from './completion/engine.ts';
import { LineTracker, echoConfirmed, looksLikeSecretPrompt } from './completion/lineTracker.ts';
import { onExternalInput } from './inputEvents';
import { InputQueue } from './inputQueue';
import { t } from './i18n';
import { TERM_FONTS, termTheme, type Appearance } from './appearance';

const useStyles = makeStyles({
  host: {
    position: 'absolute',
    inset: 0,
    padding: '10px 4px 6px 14px',
    boxSizing: 'border-box',
  },
  hidden: { visibility: 'hidden', pointerEvents: 'none' },
  pastePreview: {
    fontFamily: 'Consolas, "Cascadia Mono", monospace',
    fontSize: '13px',
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
    maxHeight: '240px',
    overflowY: 'auto',
    margin: 0,
    padding: '8px 10px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
  },
  completionMenu: {
    position: 'absolute',
    maxHeight: '226px',
    overflowY: 'auto',
    padding: '4px',
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke1}`,
    borderRadius: tokens.borderRadiusMedium,
    boxShadow: tokens.shadow8,
    boxSizing: 'border-box',
    // 一覧はキーボードだけで選ぶ。クリックは下の端末へ通し、フォーカスを奪わない。
    pointerEvents: 'none',
    zIndex: 6,
  },
  completionRow: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'baseline',
    gap: '14px',
    padding: '4px 8px',
    borderRadius: tokens.borderRadiusSmall,
    fontSize: '12px',
    whiteSpace: 'nowrap',
  },
  completionRowOn: {
    backgroundColor: tokens.colorNeutralBackground1Selected,
    color: tokens.colorNeutralForeground1Selected,
  },
  completionWord: { fontFamily: 'Consolas, "Cascadia Mono", monospace' },
  completionDesc: {
    color: tokens.colorNeutralForeground3,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
  },
});

// 補完一覧の見せかけの大きさ(配置の計算に使う)。実際の行の高さは completionRow 側。
const MENU_ROW = 27;
const MENU_MAX_H = MENU_ROW * 8 + 10;
const MENU_W = 340;

/** 貼り付ける文字から、端末の操作として働く制御文字(ESC・C1など)を除く。改行・タブは残す。
 *  ESCが残ると「貼り付けモードの終わり」を偽装して、貼り付けた途中でコマンドを実行できてしまう。 */
export function cleanPaste(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, '');
}

/** Ctrl+Spaceで開く補完の一覧。位置は開いた時点のカーソルに合わせる。 */
interface MenuView {
  items: Candidate[];
  index: number;
  left: number;
  top: number;
  width: number;
}

interface Props {
  api: Api;
  active: boolean;
  dark: boolean;
  appearance: Appearance;
  /** コマンド補完の設定(開いている端末にも反映される)。 */
  completion: boolean;
  /** 端末の大きさが決まってから呼ばれる。出力はsinkへ渡す。 */
  start: (
    size: { cols: number; rows: number },
    sink: (bytes: Uint8Array) => void,
  ) => Promise<SessionInfo>;
  onOpened: (info: SessionInfo) => void;
  onError: (message: string) => void;
}

const fontStack = (id: string) => (TERM_FONTS.find((f) => f.id === id) ?? TERM_FONTS[0]).stack;

export function TerminalView({ api, active, dark, appearance, completion, start, onOpened, onError }: Props) {
  const styles = useStyles();
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<string | null>(null);
  const trackerRef = useRef<LineTracker | null>(null);
  // 開いている補完一覧。キー操作は mount 時の1回だけ登録されるので、最新の状態は menuRef を見る。
  const [menu, setMenu] = useState<MenuView | null>(null);
  const menuRef = useRef<MenuView | null>(null);
  const compCtl = useRef<{ hideAll(): void } | null>(null);
  // 複数行の貼り付けは、確認してから端末へ送る。
  const [pasteAsk, setPasteAsk] = useState<string | null>(null);
  // 親の再描画で関数が変わっても、マウント時の1回だけ開始する。
  const latest = useRef({ start, onOpened, onError, dark, appearance, completion, active });
  latest.current = { start, onOpened, onError, dark, appearance, completion, active };
  const setMenuBoth = (next: MenuView | null) => {
    menuRef.current = next;
    setMenu(next);
  };

  // 外観の変更は、作り直さずに反映する。フォントが変わると1文字の大きさも変わるので、列数・行数を取り直して伝える。
  useEffect(() => {
    const terminal = term.current;
    if (!terminal) return;
    terminal.options.theme = termTheme(appearance.termScheme, dark, appearance.accent);
    terminal.options.fontFamily = fontStack(appearance.termFont);
    terminal.options.fontSize = appearance.termSize;
    // 表示中の候補は古い位置・色のまま残るとずれるので消す。
    compCtl.current?.hideAll();
    const timer = window.setTimeout(() => {
      fitRef.current?.fit();
      const id = idRef.current;
      if (id) void api.resize(id, terminal.cols, terminal.rows).catch(() => {});
    }, 30);
    return () => window.clearTimeout(timer);
  }, [api, dark, appearance]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const first = latest.current.appearance;
    const terminal = new Terminal({
      fontFamily: fontStack(first.termFont),
      fontSize: first.termSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: termTheme(first.termScheme, latest.current.dark, first.accent),
    });
    term.current = terminal;
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(element);
    // いま打っている行の推定。補完はエコーで確認できるときだけ出すので、追えなくなったら必ず「不明」に倒す。
    const tracker = new LineTracker();
    trackerRef.current = tracker;
    // 貼り付けは端末より先に受け取り、制御文字を除いてから送る。複数行なら確認する。
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData('text/plain') ?? '';
      event.preventDefault();
      event.stopImmediatePropagation();
      const clean = cleanPaste(text);
      if (!clean) return;
      if (clean.replace(/\r\n?/g, '\n').replace(/\n+$/, '').includes('\n')) setPasteAsk(clean);
      else {
        // terminal.paste は onData を発火するので、先に不明へ倒しておく。
        tracker.markUnknown();
        terminal.paste(clean);
      }
    };
    element.addEventListener('paste', onPaste, true);
    fit.fit();
    let id: string | null = null;
    let alive = true;
    const queue = new InputQueue(
      (data) => (id ? api.input(id, data) : Promise.resolve()),
      (message) => latest.current.onError(message),
    );

    // ---- コマンド補完 ----
    // 組み込みの辞書から、灰色の続き(ghost)とCtrl+Spaceの一覧を出す。
    // 端末へ送る入力はすべて send() を通し、表示は「画面のエコーが推定した行で終わる」
    // ことを確認してからにする。パスワード入力のように画面に出ない入力では候補を出さない。
    let shell: ShellKind | null = null;
    let ghost: string | null = null;
    const ghostEl = document.createElement('div');
    ghostEl.style.position = 'absolute';
    ghostEl.style.display = 'none';
    ghostEl.style.pointerEvents = 'none';
    ghostEl.style.whiteSpace = 'pre';
    ghostEl.style.overflow = 'hidden';
    ghostEl.style.opacity = '0.45';
    ghostEl.style.zIndex = '5';
    element.appendChild(ghostEl);

    const send = (data: string) => {
      tracker.feed(data);
      queue.push(data);
    };

    const hideGhost = () => {
      ghost = null;
      ghostEl.style.display = 'none';
    };
    const hideAll = () => {
      hideGhost();
      setMenuBoth(null);
    };
    compCtl.current = { hideAll };

    // セル1個の大きさと、.xterm-screen の host 内での位置(hostにはpaddingがあるので差分で求める)。
    const measure = () => {
      const screen = element.querySelector<HTMLElement>('.xterm-screen');
      if (!screen) return null;
      const s = screen.getBoundingClientRect();
      const h = element.getBoundingClientRect();
      return {
        x: s.left - h.left,
        y: s.top - h.top,
        cellW: s.width / terminal.cols,
        cellH: s.height / terminal.rows,
        hostW: h.width,
        hostH: h.height,
      };
    };

    // 補完を出してよい状態か。全条件を満たすときだけ推定した行とカーソル位置を返す。
    const gate = (): { line: string; x: number; y: number } | null => {
      if (!latest.current.completion || !latest.current.active || !shell) return null;
      const buffer = terminal.buffer.active;
      // vim・less などの代替画面や、アプリケーションカーソルモードでは出さない。
      if (buffer.type !== 'normal') return null;
      if (terminal.modes.applicationCursorKeysMode) return null;
      if (buffer.viewportY !== buffer.baseY) return null;
      const tracked = tracker.line;
      if (tracked === null) return null;
      const lineIndex = buffer.baseY + buffer.cursorY;
      const cursorLine = buffer.getLine(lineIndex);
      if (!cursorLine) return null;
      // カーソルの右に文字がある(行の途中にいる)ときは出さない。
      if (cursorLine.translateToString(true, buffer.cursorX) !== '') return null;
      if (buffer.getLine(lineIndex + 1)?.isWrapped) return null;
      // 折返しで繋がる前の行を遡り、カーソルより前の論理行の文字列を作る。
      let before = '';
      let row = lineIndex;
      while (row > 0 && buffer.getLine(row)?.isWrapped) {
        row -= 1;
        before = (buffer.getLine(row)?.translateToString(false) ?? '') + before;
      }
      before += cursorLine.translateToString(false, 0, buffer.cursorX);
      // 画面のエコーが推定した行で終わること。パスワード入力(エコーなし)ではここで落ちる。
      if (!echoConfirmed(before, tracked)) return null;
      if (looksLikeSecretPrompt(before)) return null;
      return { line: tracked, x: buffer.cursorX, y: buffer.cursorY };
    };

    const updateGhost = () => {
      const g = gate();
      const found = g && shell ? complete(shell, g.line).ghost : null;
      const at = g && found ? measure() : null;
      const remain = g ? terminal.cols - g.x : 0;
      if (!g || !found || !at || remain <= 0) {
        hideGhost();
        return;
      }
      ghost = found;
      ghostEl.textContent = found.slice(0, remain);
      ghostEl.style.fontFamily = terminal.options.fontFamily ?? '';
      ghostEl.style.fontSize = `${terminal.options.fontSize ?? 14}px`;
      ghostEl.style.left = `${at.x + g.x * at.cellW}px`;
      ghostEl.style.top = `${at.y + g.y * at.cellH}px`;
      ghostEl.style.height = `${at.cellH}px`;
      ghostEl.style.lineHeight = `${at.cellH}px`;
      const look = latest.current;
      const fg = termTheme(look.appearance.termScheme, look.dark, look.appearance.accent).foreground;
      if (fg) ghostEl.style.color = fg;
      ghostEl.style.display = 'block';
    };

    const openMenu = () => {
      const g = gate();
      const items = g && shell ? complete(shell, g.line).items : [];
      const at = g && items.length > 0 ? measure() : null;
      if (!g || items.length === 0 || !at) return;
      const width = Math.min(MENU_W, Math.max(160, at.hostW - 8));
      let left = at.x + g.x * at.cellW;
      if (left + width > at.hostW) left = Math.max(4, at.hostW - width - 4);
      // カーソルの下に出し、入らなければカーソル行の上へ。
      let top = at.y + (g.y + 1) * at.cellH;
      if (top + MENU_MAX_H > at.hostH) top = at.y + g.y * at.cellH - MENU_MAX_H;
      if (top < 0) top = 4;
      setMenuBoth({ items, index: 0, left, top, width });
    };

    const input = terminal.onData((data) => {
      send(data);
      // キー入力の直後は ghost を消すだけ。次の出力のあとに改めて候補を作る。
      hideGhost();
    });

    terminal.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown' || e.isComposing) return true;
      const open = menuRef.current;
      if (open) {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          const delta = e.key === 'ArrowDown' ? 1 : -1;
          setMenuBoth({ ...open, index: (open.index + delta + open.items.length) % open.items.length });
          e.preventDefault();
          return false;
        }
        if (e.key === 'Enter' || e.key === 'Tab') {
          const item = open.items[open.index];
          setMenuBoth(null);
          if (item) send(item.insert);
          e.preventDefault();
          return false;
        }
        if (e.key === 'Escape') {
          setMenuBoth(null);
          e.preventDefault();
          return false;
        }
        // それ以外のキーは一覧を閉じてから、いつもどおり端末へ渡す。
        setMenuBoth(null);
        return true;
      }
      // 一覧を開くのは Ctrl+Space だけ。Tab・Enter・矢印は端末(リモートのシェル)のものとして奪わない。
      // 補完がOFFのときは Ctrl+Space もシェルへ渡す(readline の set-mark など)。
      if (
        latest.current.completion &&
        e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey &&
        !e.metaKey &&
        (e.key === ' ' || e.code === 'Space')
      ) {
        e.preventDefault();
        openMenu();
        return false;
      }
      if (
        ghost !== null &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey &&
        !e.metaKey &&
        (e.key === 'ArrowRight' || e.key === 'End')
      ) {
        const insert = ghost;
        hideGhost();
        send(insert);
        e.preventDefault();
        return false;
      }
      return true;
    });

    let timer: number | undefined;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (element.clientWidth === 0 || element.clientHeight === 0) return;
        // 表示中の候補は古いセル位置のままなので消す。
        hideAll();
        fit.fit();
        if (id) void api.resize(id, terminal.cols, terminal.rows).catch(() => {});
      }, 40);
    });
    observer.observe(element);
    // 画面外へフォーカスが移った・スクロールバックを見た・代替画面へ入ったときも閉じる。
    const onFocusOut = () => hideAll();
    element.addEventListener('focusout', onFocusOut);
    const scrolled = terminal.onScroll(() => hideAll());
    const bufferChanged = terminal.buffer.onBufferChange(() => hideAll());
    // スニペット送信など onData を通らない入力は、別経路で「不明」へ倒す。
    const offExternal = onExternalInput((sessionId) => {
      if (sessionId === idRef.current) tracker.markUnknown();
    });
    latest.current
      .start({ cols: terminal.cols, rows: terminal.rows }, (bytes) => {
        if (!alive) return;
        // 出力の描画が終わってから候補を作る(画面のエコーを見て判断するため)。
        terminal.write(bytes, () => {
          if (alive) updateGhost();
        });
      })
      .then((info) => {
        id = info.id;
        idRef.current = info.id;
        // セッションの種類(辞書)が決まるまでは候補を出さない。
        shell = info.shell;
        tracker.reset();
        if (alive) latest.current.onOpened(info);
      })
      .catch((error) => latest.current.onError(String(error)));
    return () => {
      alive = false;
      window.clearTimeout(timer);
      observer.disconnect();
      element.removeEventListener('paste', onPaste, true);
      element.removeEventListener('focusout', onFocusOut);
      offExternal();
      scrolled.dispose();
      bufferChanged.dispose();
      input.dispose();
      ghostEl.remove();
      terminal.dispose();
      term.current = null;
      fitRef.current = null;
      idRef.current = null;
      trackerRef.current = null;
      menuRef.current = null;
      compCtl.current = null;
    };
    // 端末は1つの画面につき1回だけ作る。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (active) {
      term.current?.focus();
    } else {
      compCtl.current?.hideAll();
    }
  }, [active]);

  // 補完をOFFにしたら、表示中の候補もすぐ消す(次の出力を待たない)。
  useEffect(() => {
    if (!completion) compCtl.current?.hideAll();
  }, [completion]);

  const lines = pasteAsk ? pasteAsk.replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n').length : 0;
  return (
    <>
      <div
        ref={host}
        className={mergeClasses(styles.host, !active && styles.hidden)}
        style={{ backgroundColor: termTheme(appearance.termScheme, dark, appearance.accent).background }}
      >
        {menu && (
          <div
            className={styles.completionMenu}
            style={{ left: menu.left, top: menu.top, width: menu.width }}
            role="listbox"
            aria-label={t('コマンドの候補')}
          >
            {menu.items.map((item, index) => (
              <div
                key={`${item.word}-${index}`}
                ref={index === menu.index ? (el) => el?.scrollIntoView({ block: 'nearest' }) : undefined}
                className={mergeClasses(styles.completionRow, index === menu.index && styles.completionRowOn)}
                role="option"
                aria-selected={index === menu.index}
              >
                <span className={styles.completionWord}>{item.word}</span>
                {item.description !== '' && <span className={styles.completionDesc}>{t(item.description)}</span>}
              </div>
            ))}
          </div>
        )}
      </div>
      <Dialog open={pasteAsk !== null} onOpenChange={(_, d) => !d.open && setPasteAsk(null)}>
        <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
          <DialogBody>
            <DialogTitle>{t('{lines}行を貼り付けますか？', { lines })}</DialogTitle>
            <DialogContent style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <Caption1>{t('各行がコマンドとして実行されることがあります。内容を確認してください。')}</Caption1>
              <pre className={styles.pastePreview}>{pasteAsk}</pre>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setPasteAsk(null)}>{t('やめる')}</Button>
              <Button
                appearance="primary"
                onClick={() => {
                  const text = pasteAsk;
                  setPasteAsk(null);
                  if (text) {
                    trackerRef.current?.markUnknown();
                    term.current?.paste(text);
                  }
                  term.current?.focus();
                }}
              >
                {t('貼り付ける')}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </>
  );
}
