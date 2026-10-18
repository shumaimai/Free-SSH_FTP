import { useMemo, useState } from 'react';
import {
  Avatar,
  Badge,
  Body1,
  Button,
  Card,
  CardHeader,
  Caption1,
  makeStyles,
  SearchBox,
  Subtitle1,
  Title2,
} from '@fluentui/react-components';
import { Add24Regular, ArrowImport20Regular, DocumentEdit20Regular, Settings20Regular, Window24Regular } from '@fluentui/react-icons';
import { N_, t } from './i18n';
import type { Profile } from './types';

const useStyles = makeStyles({
  page: { padding: '24px 32px', overflowY: 'auto', height: '100%', boxSizing: 'border-box' },
  head: { display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '8px' },
  grow: { flex: 1 },
  quick: { display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '20px' },
  grid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))', gap: '12px' },
  card: { cursor: 'pointer' },
  meta: { display: 'flex', gap: '6px', marginTop: '6px' },
  empty: { padding: '28px', textAlign: 'center' },
});

const AUTH_LABEL: Record<Profile['auth_method'], string> = {
  key: N_('秘密鍵'),
  password: N_('パスワード'),
  agent: 'SSH agent',
};

export function matching(profiles: Profile[], query: string): number[] {
  const q = query.trim().toLowerCase();
  return profiles
    .map((p, i) => [p, i] as const)
    .filter(([p]) => !q || [p.name, p.host, p.username].some((f) => f.toLowerCase().includes(q)))
    .map(([, i]) => i);
}

interface Props {
  profiles: Profile[];
  disabled: boolean;
  onConnect(index: number): void;
  onEdit(index: number): void;
  onNew(): void;
  onLocal(): void;
  onImport(): void;
}

export function Home({ profiles, disabled, onConnect, onEdit, onNew, onLocal, onImport }: Props) {
  const styles = useStyles();
  const [query, setQuery] = useState('');
  const shown = useMemo(() => matching(profiles, query), [profiles, query]);
  return (
    <div className={styles.page}>
      <div className={styles.head}>
        <Title2>{t('接続')}</Title2>
        <div className={styles.grow} />
        <SearchBox
          placeholder={t('名前・ホストで検索')}
          value={query}
          onChange={(_, d) => setQuery(d.value)}
          style={{ width: 240 }}
        />
        <Button appearance="primary" icon={<Add24Regular />} disabled={disabled} onClick={onNew}>
          {t('新しい接続')}
        </Button>
      </div>
      <div className={styles.quick}>
        <Caption1>{t('すぐ使う')}</Caption1>
        <Button icon={<Window24Regular />} onClick={onLocal}>
          {t('ローカル端末')}
        </Button>
        <Button icon={<ArrowImport20Regular />} disabled={disabled} onClick={onImport}>
          {t('旧版から取り込む')}
        </Button>
      </div>
      {profiles.length === 0 ? (
        <Card className={styles.empty}>
          <Subtitle1>{t('まだ接続先がありません')}</Subtitle1>
          <Body1>
            {t('ホスト名と利用者名を入れるだけで登録できます。パスワードや秘密鍵は接続するときに入力します。旧版（Python）をお使いだった場合は、「旧版から取り込む」で接続先や保存済みのパスワードを引き継げます。')}
          </Body1>
        </Card>
      ) : shown.length === 0 ? (
        <Body1>{t('該当する接続先がありません')}</Body1>
      ) : (
        <div className={styles.grid}>
          {shown.map((index) => {
            const p = profiles[index];
            const title = p.name || p.host;
            return (
              <Card
                key={index}
                className={styles.card}
                onClick={() => onConnect(index)}
                onKeyDown={(e) => e.key === 'Enter' && onConnect(index)}
                tabIndex={0}
              >
                <CardHeader
                  image={<Avatar name={title} initials={Array.from(title)[0]?.toUpperCase()} color="colorful" size={40} />}
                  header={<Subtitle1>{title}</Subtitle1>}
                  description={
                    <Caption1>
                      {p.username}@{p.host}:{p.port}
                    </Caption1>
                  }
                  action={
                    <Button
                      appearance="subtle"
                      aria-label={t('編集')}
                      icon={<DocumentEdit20Regular />}
                      onClick={(e) => {
                        e.stopPropagation();
                        onEdit(index);
                      }}
                    />
                  }
                />
                <div className={styles.meta}>
                  <Badge appearance="tint" color="brand">
                    {t(AUTH_LABEL[p.auth_method])}
                  </Badge>
                  {p.proxy_jump && (
                    <Badge appearance="tint" color="warning" icon={<Settings20Regular />}>
                      {t('踏み台')}
                    </Badge>
                  )}
                </div>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}

