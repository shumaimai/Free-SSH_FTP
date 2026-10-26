// 2ペインの左側: このPCのフォルダ。開いているフォルダはRust側が持ち、送る項目は名前だけで依頼する。
import { Button, Caption1, Checkbox, Tooltip, Input, Menu, MenuItem, MenuList, MenuPopover, MenuTrigger, Spinner, makeStyles, mergeClasses, MessageBar, MessageBarBody, tokens } from '@fluentui/react-components';
import { Icon } from './icons';
import type { FileEntry } from './types';
import { humanSize } from './FileDock';
import { t } from './i18n';

const useStyles = makeStyles({
  root: { display: 'flex', flexDirection: 'column', gap: '8px', minHeight: 0, minWidth: 0, height: '100%' },
  row: { display: 'flex', alignItems: 'center', gap: '6px' },
  wrap: { display: 'flex', flexWrap: 'wrap', gap: '6px', alignItems: 'center' },
  grow: { flex: 1, minWidth: 0 },
  list: {
    flex: 1,
    minHeight: 0,
    overflowY: 'auto',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  item: {
    display: 'flex',
    alignItems: 'center',
    gap: '8px',
    padding: '5px 10px',
    cursor: 'default',
    userSelect: 'none',
    touchAction: 'none',
    ':hover': { backgroundColor: tokens.colorNeutralBackground1Hover },
  },
  itemOn: { backgroundColor: tokens.colorBrandBackground2, ':hover': { backgroundColor: tokens.colorBrandBackground2Hover } },
  name: { flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  dir: { color: tokens.colorBrandForeground1 },
  size: { color: tokens.colorNeutralForeground3, flexShrink: 0, fontSize: '12px' },
  note: { color: tokens.colorNeutralForeground3 },
});

interface Props {
  path: string;
  pathInput: string;
  parent: string | null;
  entries: FileEntry[];
  selected: Set<string>;
  hidden: boolean;
  loading: boolean;
  error: string | null;
  sync: boolean;
  roots: string[];
  canSend: boolean;
  onPathInput(value: string): void;
  onGo(path: string): void;
  onRefresh(): void;
  onPick(): void;
  onRoots(): void;
  onEnter(entry: FileEntry): void;
  onSelect(entry: FileEntry, additive: boolean, range: boolean): void;
  onRowPointerDown(entry: FileEntry, event: React.PointerEvent): void;
  onHidden(value: boolean): void;
  onSync(value: boolean): void;
  onSend(): void;
}

export function LocalPane(p: Props) {
  const styles = useStyles();
  return (
    <div className={styles.root} data-pane="local">
      <div className={styles.row}>
        <Tooltip content={t('ローカルの親フォルダへ')} relationship="label">
          <Button size="small" icon={<Icon.parentFolder />} disabled={!p.parent} onClick={() => p.parent && p.onGo(p.parent)} />
        </Tooltip>
        <Tooltip content={t('ローカルを再読み込み')} relationship="label">
          <Button size="small" icon={<Icon.refresh />} onClick={p.onRefresh} />
        </Tooltip>
        <Input
          className={styles.grow}
          size="small"
          aria-label={t('ローカルのフォルダのパス')}
          value={p.pathInput}
          onChange={(_, d) => p.onPathInput(d.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) p.onGo(p.pathInput);
          }}
        />
        <Menu onOpenChange={(_, d) => d.open && p.onRoots()}>
          <MenuTrigger disableButtonEnhancement>
            <Tooltip content={t('ドライブ・ルート')} relationship="label">
              <Button size="small" icon={<Icon.drive />} />
            </Tooltip>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              {p.roots.map((root) => (
                <MenuItem key={root} onClick={() => p.onGo(root)}>
                  {root}
                </MenuItem>
              ))}
            </MenuList>
          </MenuPopover>
        </Menu>
      </div>
      <div className={styles.wrap}>
        <Button appearance="primary" size="small" icon={<Icon.upload />} disabled={!p.canSend || p.selected.size === 0} onClick={p.onSend}>
          {t('リモートへ送信')}
        </Button>
        <Button size="small" icon={<Icon.pickFolder />} onClick={p.onPick}>
          {t('フォルダを選ぶ')}
        </Button>
        <div className={styles.grow} />
        <Checkbox checked={p.hidden} onChange={(_, d) => p.onHidden(d.checked === true)} label={t('隠しファイル')} />
      </div>
      <Checkbox
        checked={p.sync}
        onChange={(_, d) => p.onSync(d.checked === true)}
        label={t('同期ブラウズ（片側のフォルダ移動を相手側へ相対的に写す）')}
      />
      {p.error && (
        <MessageBar intent="error">
          <MessageBarBody>{p.error}</MessageBarBody>
        </MessageBar>
      )}
      <div className={styles.list} role="listbox" aria-multiselectable="true" aria-label={t('ローカルのファイル一覧')}>
        {p.loading && p.entries.length === 0 && (
          <div style={{ padding: 12 }}>
            <Spinner size="tiny" label={t('読み込み中')} />
          </div>
        )}
        {!p.loading && p.entries.length === 0 && (
          <Caption1 className={styles.note} style={{ padding: 12, display: 'block' }}>
            {t('空のフォルダです')}
          </Caption1>
        )}
        {p.entries.map((entry) => (
          <div
            key={entry.path}
            role="option"
            aria-selected={p.selected.has(entry.name)}
            tabIndex={0}
            className={mergeClasses(styles.item, p.selected.has(entry.name) && styles.itemOn)}
            onPointerDown={(e) => p.onRowPointerDown(entry, e)}
            onClick={(e) => p.onSelect(entry, e.ctrlKey || e.metaKey, e.shiftKey)}
            onDoubleClick={() => p.onEnter(entry)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') p.onEnter(entry);
              if (e.key === ' ') {
                e.preventDefault();
                p.onSelect(entry, true, false);
              }
            }}
          >
            {entry.directory ? <Icon.folder className={styles.dir} /> : <Icon.file />}
            <span className={mergeClasses(styles.name, entry.directory && styles.dir)} title={entry.name}>
              {entry.name}
              {entry.symlink && ' ' + t('（リンク）')}
            </span>
            {!entry.directory && <span className={styles.size}>{humanSize(entry.size)}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}
