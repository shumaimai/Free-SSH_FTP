// 描画中の例外で画面全体が真っ白にならないよう、原因を表示して記録する。
import { Component, type ErrorInfo, type ReactNode } from 'react';
import { t } from './i18n';
import { reportClientError } from './report';

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    reportClientError(`描画エラー: ${error.message}\n${error.stack ?? ''}\n${info.componentStack ?? ''}`); // i18n-ignore: 開発者向けの記録(ログ)
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div style={{ padding: 24, fontFamily: 'system-ui, sans-serif', maxWidth: 720 }}>
        <h2>{t('画面の表示でエラーが起きました')}</h2>
        <p>{t('接続中のセッションや実行中の作業は、画面の外（バックグラウンド）で続いている場合があります。「画面を読み込み直す」で表示を戻せます。')}</p>
        <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', background: '#f3f3f3', padding: 12, color: '#b00020' }}>
          {this.state.error.message}
        </pre>
        <p>{t('詳細は設定フォルダの client-errors.log に記録しました。')}</p>
        <button onClick={() => window.location.reload()}>{t('画面を読み込み直す')}</button>
      </div>
    );
  }
}
