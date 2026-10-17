// 内蔵エディタ。文章はCodeMirror 6、バイナリは上書き専用の16進グリッド（サイズは変えない）。
// 保存先のパスや外部変更の照合はRust側が持つ。ここは表示と編集内容を渡すだけ。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button, Caption1, MessageBar, MessageBarBody, makeStyles, tokens } from '@fluentui/react-components';
import { Save20Regular, SaveEdit20Regular } from '@fluentui/react-icons';
import { EditorState, Compartment, type Extension, type Text } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { basicSetup } from 'codemirror';
import { indentWithTab } from '@codemirror/commands';
import { StreamLanguage, syntaxHighlighting, defaultHighlightStyle } from '@codemirror/language';
import { oneDarkHighlightStyle } from '@codemirror/theme-one-dark';
import { javascript } from '@codemirror/lang-javascript';
import { python } from '@codemirror/lang-python';
import { json } from '@codemirror/lang-json';
import { html } from '@codemirror/lang-html';
import { css } from '@codemirror/lang-css';
import { markdown } from '@codemirror/lang-markdown';
import { yaml } from '@codemirror/lang-yaml';
import { xml } from '@codemirror/lang-xml';
import { shell } from '@codemirror/legacy-modes/mode/shell';
import { toml } from '@codemirror/legacy-modes/mode/toml';
import { properties } from '@codemirror/legacy-modes/mode/properties';
import { nginx } from '@codemirror/legacy-modes/mode/nginx';
import type { Api } from './api';
import type { OpenedDoc } from './types';
import { TERM_FONTS, type Appearance } from './appearance';

const useStyles = makeStyles({
  root: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 },
  bar: {
    display: 'flex',
    alignItems: 'center',
    gap: '8px',
    padding: '6px 12px',
    backgroundColor: tokens.colorNeutralBackground3,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  grow: { flex: 1 },
  path: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 },
  body: { flex: 1, minHeight: 0, position: 'relative' },
  hex: {
    position: 'absolute',
    inset: 0,
    overflow: 'auto',
    outlineStyle: 'none',
    backgroundColor: tokens.colorNeutralBackground1,
  },
  row: { position: 'absolute', left: 0, whiteSpace: 'pre', paddingLeft: '12px' },
  off: { color: tokens.colorNeutralForeground3 },
  byte: { cursor: 'pointer', borderRadius: '2px' },
  cur: { backgroundColor: tokens.colorBrandBackground, color: tokens.colorNeutralForegroundOnBrand },
  changed: { color: tokens.colorPaletteRedForeground1, fontWeight: tokens.fontWeightBold },
});

const ROW = 22;
const COLS = 16;

function extension(name: string): Extension | null {
  const lower = name.toLowerCase();
  const ext = lower.includes('.') ? lower.slice(lower.lastIndexOf('.') + 1) : '';
  switch (ext) {
    case 'js': case 'mjs': case 'cjs': return javascript();
    case 'jsx': return javascript({ jsx: true });
    case 'ts': return javascript({ typescript: true });
    case 'tsx': return javascript({ typescript: true, jsx: true });
    case 'py': return python();
    case 'json': return json();
    case 'html': case 'htm': return html();
    case 'css': return css();
    case 'md': case 'markdown': return markdown();
    case 'yml': case 'yaml': return yaml();
    case 'xml': case 'svg': return xml();
    case 'sh': case 'bash': case 'zsh': return StreamLanguage.define(shell);
    case 'toml': return StreamLanguage.define(toml);
    case 'ini': case 'cfg': case 'properties': case 'env': return StreamLanguage.define(properties);
    case 'conf': return StreamLanguage.define(nginx);
    default:
      return lower === '.bashrc' || lower === '.profile' || lower === '.bash_profile' ? StreamLanguage.define(shell) : null;
  }
}

function decode(base64: string): Uint8Array {
  const bin = atob(base64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

interface Props {
  api: Api;
  doc: OpenedDoc;
  active: boolean;
  dark: boolean;
  appearance: Appearance;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string, path: string, sessionId: string | null) => void;
  onMessage: (message: string) => void;
}

export function EditorPane(props: Props) {
  return props.doc.kind === 'hex' ? <HexEditor {...props} /> : <TextEditor {...props} />;
}

function Bar({
  doc,
  path,
  dirty,
  busy,
  save,
  saveAs,
  extra,
}: {
  doc: OpenedDoc;
  path: string;
  dirty: boolean;
  busy: boolean;
  save: () => void;
  saveAs: () => void;
  extra: string;
}) {
  const styles = useStyles();
  return (
    <div className={styles.bar}>
      <Button appearance="primary" size="small" icon={<Save20Regular />} disabled={busy || !dirty} onClick={save}>
        保存
      </Button>
      <Button size="small" icon={<SaveEdit20Regular />} disabled={busy} onClick={saveAs}>
        名前を付けて保存...
      </Button>
      <Caption1 className={styles.path} title={path}>
        {doc.sessionId ? '[サーバー] ' : '[この PC] '}
        {path}
      </Caption1>
      <div className={styles.grow} />
      <Caption1>{extra}</Caption1>
    </div>
  );
}

function TextEditor({ api, doc, active, dark, appearance, onDirty, onSaved, onMessage }: Props) {
  const styles = useStyles();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const saved = useRef<Text | null>(null);
  const dirtyRef = useRef(false);
  const theme = useRef(new Compartment());
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [path, setPath] = useState(doc.path);
  const [error, setError] = useState('');
  const [position, setPosition] = useState('1:1');
  const callbacks = useRef({ onDirty, onSaved, onMessage });
  callbacks.current = { onDirty, onSaved, onMessage };

  const themeExt = useMemo(
    () => [
      syntaxHighlighting(dark ? oneDarkHighlightStyle : defaultHighlightStyle),
      EditorView.theme(
        {
          '&': { height: '100%', backgroundColor: 'var(--colorNeutralBackground1)', color: 'var(--colorNeutralForeground1)' },
          '.cm-content': { caretColor: 'var(--colorNeutralForeground1)' },
          '.cm-cursor': { borderLeftColor: 'var(--colorNeutralForeground1)' },
          '.cm-gutters': {
            backgroundColor: 'var(--colorNeutralBackground3)',
            color: 'var(--colorNeutralForeground3)',
            border: 'none',
          },
          '.cm-activeLine': { backgroundColor: 'var(--colorSubtleBackgroundHover)' },
          '.cm-activeLineGutter': { backgroundColor: 'var(--colorSubtleBackgroundHover)' },
          '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
            backgroundColor: 'var(--colorBrandBackground2)',
          },
          '.cm-panels': { backgroundColor: 'var(--colorNeutralBackground3)', color: 'var(--colorNeutralForeground1)' },
          '.cm-panels input, .cm-panels button': { font: 'inherit' },
          '.cm-scroller': {
            fontFamily: (TERM_FONTS.find((f) => f.id === appearance.termFont) ?? TERM_FONTS[0]).stack,
            fontSize: `${appearance.termSize}px`,
          },
        },
        { dark },
      ),
    ],
    [dark, appearance.termFont, appearance.termSize],
  );

  const doSave = useCallback(
    async (as: boolean) => {
      if (!view.current || busy) return;
      setBusy(true);
      setError('');
      const snapshot = view.current.state.doc;
      const text = snapshot.toString();
      try {
        const result = as ? await api.editorSaveAs(doc.id, text, null) : await api.editorSave(doc.id, text, null);
        if (result) {
          saved.current = snapshot;
          const now = !view.current.state.doc.eq(snapshot);
          dirtyRef.current = now;
          setDirty(now);
          void api.editorSetDirty(doc.id, now);
          callbacks.current.onDirty(now);
          setPath(result.path);
          callbacks.current.onSaved(result.name, result.path, result.sessionId);
          callbacks.current.onMessage(`${result.name} を保存しました`);
        }
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [api, doc.id, busy],
  );
  const saveRef = useRef(doSave);
  saveRef.current = doSave;

  useEffect(() => {
    if (!host.current) return;
    const language = extension(doc.name);
    const initial = EditorState.create({
      doc: doc.text ?? '',
      extensions: [
        basicSetup,
        keymap.of([
          indentWithTab,
          {
            key: 'Mod-s',
            preventDefault: true,
            run: () => {
              void saveRef.current(false);
              return true;
            },
          },
        ]),
        language ?? [],
        theme.current.of(themeExt),
        EditorView.updateListener.of((update) => {
          if (update.selectionSet || update.docChanged) {
            const head = update.state.selection.main.head;
            const line = update.state.doc.lineAt(head);
            setPosition(`${line.number}:${head - line.from + 1}`);
          }
          if (update.docChanged && saved.current) {
            const now = !update.state.doc.eq(saved.current);
            if (now !== dirtyRef.current) {
              dirtyRef.current = now;
              setDirty(now);
              void api.editorSetDirty(doc.id, now);
              callbacks.current.onDirty(now);
            }
          }
        }),
      ],
    });
    const editor = new EditorView({ state: initial, parent: host.current });
    saved.current = editor.state.doc;
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
    // 文書ごとに作り直す。外観の変更は下のeffectで差し替える。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc.id]);

  useEffect(() => {
    view.current?.dispatch({ effects: theme.current.reconfigure(themeExt) });
  }, [themeExt]);

  useEffect(() => {
    if (active) view.current?.focus();
  }, [active]);

  return (
    <div className={styles.root}>
      <Bar
        doc={doc}
        path={path}
        dirty={dirty}
        busy={busy}
        save={() => void doSave(false)}
        saveAs={() => void doSave(true)}
        extra={`${doc.encoding} / ${doc.newline} / ${position}`}
      />
      {error && (
        <MessageBar intent="error">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}
      <div className={styles.body} ref={host} style={{ overflow: 'hidden' }} />
    </div>
  );
}

function HexEditor({ api, doc, active, appearance, onDirty, onSaved, onMessage }: Props) {
  const styles = useStyles();
  const original = useRef<Uint8Array>(useMemo(() => decode(doc.hex ?? ''), [doc.hex]));
  const [patches, setPatches] = useState<Map<number, number>>(new Map());
  const [cursor, setCursor] = useState(0);
  const [nibble, setNibble] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [path, setPath] = useState(doc.path);
  const [error, setError] = useState('');
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(400);
  const box = useRef<HTMLDivElement>(null);
  const bytes = original.current;
  const rows = Math.max(1, Math.ceil(bytes.length / COLS));
  const dirty = patches.size > 0;
  const font = (TERM_FONTS.find((f) => f.id === appearance.termFont) ?? TERM_FONTS[0]).stack;

  useEffect(() => {
    onDirty(dirty);
    void api.editorSetDirty(doc.id, dirty);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dirty]);
  useEffect(() => {
    if (active) box.current?.focus();
  }, [active]);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setHeight(el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const valueAt = (i: number) => patches.get(i) ?? bytes[i];
  const move = (next: number) => {
    const index = Math.max(0, Math.min(bytes.length - 1, next));
    setCursor(index);
    setNibble(null);
    const el = box.current;
    if (el) {
      const top = Math.floor(index / COLS) * ROW;
      if (top < el.scrollTop) el.scrollTop = top;
      else if (top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW - el.clientHeight;
    }
  };
  const setByte = (index: number, value: number) =>
    setPatches((old) => {
      const next = new Map(old);
      if (value === bytes[index]) next.delete(index);
      else next.set(index, value);
      return next;
    });

  const onKey = (e: React.KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      void save(false);
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const pageRows = Math.max(1, Math.floor(height / ROW) - 1);
    switch (e.key) {
      case 'ArrowLeft': move(cursor - 1); break;
      case 'ArrowRight': move(cursor + 1); break;
      case 'ArrowUp': move(cursor - COLS); break;
      case 'ArrowDown': move(cursor + COLS); break;
      case 'PageUp': move(cursor - COLS * pageRows); break;
      case 'PageDown': move(cursor + COLS * pageRows); break;
      case 'Home': move(cursor - (cursor % COLS)); break;
      case 'End': move(cursor - (cursor % COLS) + COLS - 1); break;
      case 'Escape': setNibble(null); return;
      default:
        if (/^[0-9a-fA-F]$/.test(e.key) && bytes.length > 0) {
          if (nibble === null) {
            setNibble(e.key);
            // 1桁目は仮に上位だけ反映して見せる。
            setByte(cursor, (parseInt(e.key, 16) << 4) | (valueAt(cursor) & 0x0f));
          } else {
            setByte(cursor, parseInt(nibble + e.key, 16));
            setNibble(null);
            if (cursor + 1 < bytes.length) move(cursor + 1);
          }
          break;
        }
        return;
    }
    e.preventDefault();
  };

  const save = async (as: boolean) => {
    if (busy) return;
    setBusy(true);
    setError('');
    const sent = new Map(patches);
    const list = [...sent].map(([o, v]) => [o, v] as [number, number]);
    try {
      const result = as ? await api.editorSaveAs(doc.id, null, list) : await api.editorSave(doc.id, null, list);
      if (result) {
        sent.forEach((v, o) => (bytes[o] = v));
        setPatches((old) => {
          const next = new Map(old);
          sent.forEach((v, o) => {
            if (next.get(o) === v) next.delete(o);
          });
          return next;
        });
        setNibble(null);
        setPath(result.path);
        onSaved(result.name, result.path, result.sessionId);
        onMessage(`${result.name} を保存しました`);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const first = Math.max(0, Math.floor(scroll / ROW) - 2);
  const last = Math.min(rows, Math.ceil((scroll + height) / ROW) + 2);
  const lines = [];
  for (let r = first; r < last; r++) {
    const cells = [];
    let ascii = '';
    for (let c = 0; c < COLS; c++) {
      const i = r * COLS + c;
      if (i >= bytes.length) {
        cells.push(<span key={c}>{'   '}</span>);
        continue;
      }
      const v = valueAt(i);
      ascii += v >= 0x20 && v < 0x7f ? String.fromCharCode(v) : '.';
      const classes = [styles.byte, i === cursor ? styles.cur : '', patches.has(i) ? styles.changed : ''].join(' ');
      cells.push(
        <span key={c}>
          <span className={classes} onClick={() => { setCursor(i); setNibble(null); box.current?.focus(); }}>
            {v.toString(16).padStart(2, '0')}
          </span>
          {c === 7 ? '  ' : ' '}
        </span>,
      );
    }
    lines.push(
      <div key={r} className={styles.row} style={{ top: r * ROW, height: ROW, lineHeight: `${ROW}px` }}>
        <span className={styles.off}>{(r * COLS).toString(16).padStart(8, '0')}</span>
        {'  '}
        {cells}
        {' '}
        <span className={styles.off}>{ascii}</span>
      </div>,
    );
  }

  return (
    <div className={styles.root}>
      <Bar
        doc={doc}
        path={path}
        dirty={dirty}
        busy={busy}
        save={() => void save(false)}
        saveAs={() => void save(true)}
        extra={`16進 / サイズ不変 / 位置 0x${cursor.toString(16)} / 変更 ${patches.size}バイト`}
      />
      {error && (
        <MessageBar intent="error">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}
      <div className={styles.body}>
        <div
          ref={box}
          tabIndex={0}
          role="textbox"
          aria-label="16進エディタ"
          className={styles.hex}
          style={{ fontFamily: font, fontSize: `${appearance.termSize}px` }}
          onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
          onKeyDown={onKey}
        >
          <div style={{ position: 'relative', height: rows * ROW, minWidth: '100%' }}>{lines}</div>
        </div>
      </div>
    </div>
  );
}
