import { useEffect, useState } from 'react';
import {
  Button,
  Caption1,
  Checkbox,
  Input,
  Tab,
  TabList,
  MessageBar,
  MessageBarBody,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  makeStyles,
  mergeClasses,
  Radio,
  RadioGroup,
  Select,
  Slider,
  tokens,
} from '@fluentui/react-components';
import { Checkmark16Filled } from '@fluentui/react-icons';
import type { Api } from './api';
import type { AppSettings } from './types';
import {
  ACCENTS,
  DEFAULT_APPEARANCE,
  MODES,
  TERM_FONTS,
  TERM_SCHEMES,
  UI_FONTS,
  termTheme,
  type Appearance,
  type ThemeMode,
} from './appearance';

const useStyles = makeStyles({
  body: { display: 'flex', flexDirection: 'column', gap: '16px' },
  swatches: { display: 'flex', gap: '10px', flexWrap: 'wrap' },
  swatch: {
    width: '28px',
    height: '28px',
    borderRadius: '50%',
    border: `2px solid transparent`,
    cursor: 'pointer',
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    color: '#fff',
    padding: 0,
  },
  swatchOn: { outline: `2px solid ${tokens.colorNeutralForeground1}`, outlineOffset: '2px' },
  sizeRow: { display: 'flex', alignItems: 'center', gap: '12px' },
  preview: {
    borderRadius: tokens.borderRadiusMedium,
    padding: '10px 12px',
    whiteSpace: 'pre',
    lineHeight: 1.35,
    overflow: 'hidden',
  },
});

interface Props {
  api: Api;
  open: boolean;
  value: Appearance;
  dark: boolean;
  onChange(value: Appearance): void;
  onClose(): void;
}

export function SettingsDialog({ api, open, value, dark, onChange, onClose }: Props) {
  const styles = useStyles();
  const [tab, setTab] = useState<'look' | 'general'>('look');
  const [general, setGeneral] = useState<AppSettings | null>(null);
  const [generalError, setGeneralError] = useState<string | null>(null);
  useEffect(() => {
    if (open) {
      setTab('look');
      setGeneralError(null);
      void api.settingsGet().then(setGeneral);
    }
  }, [open, api]);
  const saveGeneral = (next: AppSettings) => {
    setGeneral(next);
    void api.settingsSet(next).then(() => setGeneralError(null)).catch((e) => setGeneralError(String(e)));
  };
  const set = <K extends keyof Appearance>(key: K, v: Appearance[K]) => onChange({ ...value, [key]: v });
  const term = termTheme(value.termScheme, dark, value.accent);
  const font = TERM_FONTS.find((f) => f.id === value.termFont)?.stack;
  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
        <DialogBody>
          <DialogTitle>設定</DialogTitle>
          <DialogContent className={styles.body}>
          <TabList selectedValue={tab} onTabSelect={(_, d) => setTab(d.value as 'look' | 'general')}>
            <Tab value="look">外観</Tab>
            <Tab value="general">全般</Tab>
          </TabList>
          {tab === 'general' && general && (
            <>
              <Checkbox
                checked={general.updateCheck}
                onChange={(_, d) => saveGeneral({ ...general, updateCheck: d.checked === true })}
                label="起動時に新しいプレビュー版を確認する"
              />
              <Caption1>旧Python版のリリースとは別に、rust-preview-v のタグだけを確認します（次の起動から）。</Caption1>
              <Checkbox
                checked={general.sessionLog}
                onChange={(_, d) => saveGeneral({ ...general, sessionLog: d.checked === true })}
                label="SSH端末の受信出力を自動保存する（PuTTYのloggingに相当）"
              />
              <Field label="保存先（空なら設定フォルダ/logs）">
                <div style={{ display: 'flex', gap: 8 }}>
                  <Input
                    style={{ flex: 1 }}
                    disabled={!general.sessionLog}
                    value={general.sessionLogDir}
                    onChange={(_, d) => saveGeneral({ ...general, sessionLogDir: d.value })}
                  />
                  <Button
                    disabled={!general.sessionLog}
                    onClick={async () => {
                      const dir = await api.settingsPickLogDir();
                      if (dir) saveGeneral({ ...general, sessionLogDir: dir });
                    }}
                  >
                    参照
                  </Button>
                </div>
              </Field>
              <Caption1>改行で確定した行だけを記録します。vimなどの全画面表示は記録しません。新しい接続から適用します。</Caption1>
              {generalError && (
                <MessageBar intent="error">
                  <MessageBarBody>{generalError}</MessageBarBody>
                </MessageBar>
              )}
            </>
          )}
          <div className={styles.body} style={{ display: tab === 'look' ? undefined : 'none' }}>
            <Field label="テーマ">
              <RadioGroup layout="horizontal" value={value.mode} onChange={(_, d) => set('mode', d.value as ThemeMode)}>
                {MODES.map((m) => (
                  <Radio key={m.id} value={m.id} label={m.label} />
                ))}
              </RadioGroup>
            </Field>
            <Field label="アクセント色">
              <div className={styles.swatches} role="radiogroup" aria-label="アクセント色">
                {ACCENTS.map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    role="radio"
                    aria-checked={value.accent === a.id}
                    aria-label={a.label}
                    title={a.label}
                    className={mergeClasses(styles.swatch, value.accent === a.id && styles.swatchOn)}
                    style={{ backgroundColor: a.color }}
                    onClick={() => set('accent', a.id)}
                  >
                    {value.accent === a.id && <Checkmark16Filled />}
                  </button>
                ))}
              </div>
            </Field>
            <Field label="画面のフォント">
              <Select value={value.uiFont} onChange={(_, d) => set('uiFont', d.value)}>
                {UI_FONTS.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="端末のフォント">
              <Select value={value.termFont} onChange={(_, d) => set('termFont', d.value)}>
                {TERM_FONTS.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label={`端末の文字サイズ（${value.termSize}px）`}>
              <div className={styles.sizeRow}>
                <Slider
                  style={{ flex: 1 }}
                  min={10}
                  max={24}
                  step={1}
                  value={value.termSize}
                  onChange={(_, d) => set('termSize', d.value)}
                />
              </div>
            </Field>
            <Field label="端末の配色">
              <Select value={value.termScheme} onChange={(_, d) => set('termScheme', d.value)}>
                {TERM_SCHEMES.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label}
                  </option>
                ))}
              </Select>
            </Field>
            <div
              className={styles.preview}
              style={{
                background: term.background,
                color: term.foreground,
                fontFamily: font,
                fontSize: value.termSize,
              }}
              aria-label="端末の表示例"
            >
              <span style={{ color: term.green }}>user@hashi</span>:<span style={{ color: term.blue }}>~</span>$ ls
              {'\n'}docs  <span style={{ color: term.blue }}>projects</span>  <span style={{ color: term.red }}>archive.tar.gz</span>
              {'\n'}日本語の表示例 — 橋
            </div>
            <Caption1>選ぶとすぐに反映され、次回の起動でも使われます。画面のフォントは、この機械に入っているものだけが使われます。</Caption1>
          </div>
          </DialogContent>
          <DialogActions>
            <Button onClick={() => onChange(DEFAULT_APPEARANCE)}>既定に戻す</Button>
            <Button appearance="primary" onClick={onClose}>
              閉じる
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
